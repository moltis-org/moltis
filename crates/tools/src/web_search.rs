use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use {
    async_trait::async_trait,
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
    tracing::{debug, warn},
};

use crate::error::Error;

use {
    moltis_agents::tool_registry::AgentTool,
    moltis_config::schema::{SearchProvider as ConfigSearchProvider, WebSearchConfig},
};

use crate::exec::EnvVarProvider;

mod brave;

/// Cached search result with expiry.
struct CacheEntry {
    value: serde_json::Value,
    expires_at: Instant,
}

/// Web search tool — lets the LLM search the web via Brave Search or Perplexity.
///
/// When the configured provider's API key is missing and fallback is enabled,
/// the tool falls back to DuckDuckGo HTML search.
pub struct WebSearchTool {
    provider: SearchProvider,
    configured_api_key: Secret<String>,
    max_results: u8,
    timeout: Duration,
    cache_ttl: Duration,
    cache: Mutex<HashMap<String, CacheEntry>>,
    /// Whether to fall back to DuckDuckGo when the API key is missing.
    /// Defaults to `false` in production. Tests also set this to `false` to
    /// avoid network calls.
    fallback_enabled: bool,
    /// When DuckDuckGo returns a CAPTCHA, block it until this instant so
    /// subsequent calls fail fast instead of wasting network round-trips.
    ddg_blocked_until: Mutex<Option<Instant>>,
    /// Optional runtime env provider (credential store) for hot key updates.
    env_provider: Option<Arc<dyn EnvVarProvider>>,
    /// Test-only endpoint override used to exercise the complete Brave HTTP flow.
    #[cfg(test)]
    brave_endpoint_override: Option<String>,
}

#[derive(Debug, Clone)]
enum SearchProvider {
    Brave,
    Perplexity {
        base_url_override: Option<String>,
        model: String,
    },
    #[cfg(feature = "firecrawl")]
    Firecrawl {
        base_url: String,
    },
}

fn env_value_with_overrides(env_overrides: &HashMap<String, String>, key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env_overrides
                .get(key)
                .cloned()
                .filter(|value| !value.trim().is_empty())
        })
}

/// A single Brave search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BraveResult {
    title: String,
    url: String,
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
        Self::from_config_with_env_overrides(config, &HashMap::new())
    }

    /// Build from config, with optional env overrides used as fallback after
    /// process env. This avoids process-global env mutation in callers.
    pub fn from_config_with_env_overrides(
        config: &WebSearchConfig,
        env_overrides: &HashMap<String, String>,
    ) -> Option<Self> {
        if !config.enabled {
            return None;
        }

        match config.provider {
            ConfigSearchProvider::Brave => {
                let api_key = config
                    .api_key
                    .as_ref()
                    .map(|s| s.expose_secret().clone())
                    .or_else(|| env_value_with_overrides(env_overrides, "BRAVE_API_KEY"))
                    .unwrap_or_default();
                // Don't register when no API key and no fallback — the tool
                // would always fail, confusing the LLM.
                if api_key.is_empty() && !config.duckduckgo_fallback {
                    return None;
                }
                Some(Self::new(
                    SearchProvider::Brave,
                    Secret::new(api_key),
                    config.max_results,
                    Duration::from_secs(config.timeout_seconds),
                    Duration::from_secs(config.cache_ttl_minutes * 60),
                    config.duckduckgo_fallback,
                ))
            },
            ConfigSearchProvider::Perplexity => {
                let api_key = config
                    .perplexity
                    .api_key
                    .as_ref()
                    .map(|s| s.expose_secret().clone())
                    .or_else(|| env_value_with_overrides(env_overrides, "PERPLEXITY_API_KEY"))
                    .or_else(|| env_value_with_overrides(env_overrides, "OPENROUTER_API_KEY"))
                    .unwrap_or_default();
                if api_key.is_empty() && !config.duckduckgo_fallback {
                    return None;
                }
                let base_url_override = config
                    .perplexity
                    .base_url
                    .clone()
                    .filter(|value| !value.trim().is_empty());
                let model = config
                    .perplexity
                    .model
                    .clone()
                    .unwrap_or_else(|| "perplexity/sonar-pro".into());
                Some(Self::new(
                    SearchProvider::Perplexity {
                        base_url_override,
                        model,
                    },
                    Secret::new(api_key),
                    config.max_results,
                    Duration::from_secs(config.timeout_seconds),
                    Duration::from_secs(config.cache_ttl_minutes * 60),
                    config.duckduckgo_fallback,
                ))
            },
            // Firecrawl search: the API key may come from several sources:
            //   1. tools.web.search.api_key (checked here)
            //   2. FIRECRAWL_API_KEY env var (checked here)
            //   3. tools.web.firecrawl.api_key (injected later via with_firecrawl_config)
            //   4. Runtime env provider (resolved at execute time)
            // Because (3) and (4) are not available yet, we always construct
            // the tool and let execute() handle missing keys gracefully.
            #[cfg(feature = "firecrawl")]
            ConfigSearchProvider::Firecrawl => {
                let api_key = config
                    .api_key
                    .as_ref()
                    .map(|s| s.expose_secret().clone())
                    .or_else(|| env_value_with_overrides(env_overrides, "FIRECRAWL_API_KEY"))
                    .unwrap_or_default();
                Some(Self::new(
                    SearchProvider::Firecrawl {
                        base_url: crate::firecrawl::DEFAULT_BASE_URL.into(),
                    },
                    Secret::new(api_key),
                    config.max_results,
                    Duration::from_secs(config.timeout_seconds),
                    Duration::from_secs(config.cache_ttl_minutes * 60),
                    config.duckduckgo_fallback,
                ))
            },
            // When the firecrawl feature is disabled but the config says
            // "firecrawl", treat it the same as missing provider — fall
            // through to None so the tool isn't registered.
            #[cfg(not(feature = "firecrawl"))]
            ConfigSearchProvider::Firecrawl => {
                warn!("search provider 'firecrawl' selected but the firecrawl feature is disabled");
                None
            },
        }
    }

    fn new(
        provider: SearchProvider,
        configured_api_key: Secret<String>,
        max_results: u8,
        timeout: Duration,
        cache_ttl: Duration,
        fallback_enabled: bool,
    ) -> Self {
        Self {
            provider,
            configured_api_key,
            max_results,
            timeout,
            cache_ttl,
            cache: Mutex::new(HashMap::new()),
            fallback_enabled,
            ddg_blocked_until: Mutex::new(None),
            env_provider: None,
            #[cfg(test)]
            brave_endpoint_override: None,
        }
    }

    #[cfg(test)]
    fn with_brave_endpoint(mut self, endpoint: String) -> Self {
        self.brave_endpoint_override = Some(endpoint);
        self
    }

    fn brave_endpoint(&self) -> &str {
        #[cfg(test)]
        if let Some(endpoint) = self.brave_endpoint_override.as_deref() {
            return endpoint;
        }

        brave::ENDPOINT
    }

    /// Attach a runtime environment provider (credential store).
    pub fn with_env_provider(mut self, provider: Arc<dyn EnvVarProvider>) -> Self {
        self.env_provider = Some(provider);
        self
    }

    /// Update the Firecrawl base URL from the shared Firecrawl config.
    /// Also picks up the API key from `FirecrawlConfig` if not already set.
    #[cfg(feature = "firecrawl")]
    #[must_use]
    pub fn with_firecrawl_config(
        mut self,
        config: &moltis_config::schema::FirecrawlConfig,
    ) -> Self {
        if let SearchProvider::Firecrawl { ref mut base_url } = self.provider {
            if !config.base_url.trim().is_empty() {
                *base_url = config.base_url.clone();
            }
            // If the configured key is empty, try the firecrawl config key.
            if self.configured_api_key.expose_secret().is_empty()
                && let Some(ref key) = config.api_key
            {
                let val = key.expose_secret();
                if !val.trim().is_empty() {
                    self.configured_api_key = key.clone();
                }
            }
        }
        self
    }

    async fn runtime_env_values(&self) -> HashMap<String, String> {
        let Some(provider) = self.env_provider.as_ref() else {
            return HashMap::new();
        };

        provider
            .get_env_vars()
            .await
            .into_iter()
            .filter_map(|(key, value)| {
                let value = value.expose_secret().clone();
                if key.trim().is_empty() || value.trim().is_empty() {
                    None
                } else {
                    Some((key, value))
                }
            })
            .collect()
    }

    fn lookup_env_value(env_values: &HashMap<String, String>, key: &str) -> Option<String> {
        std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                env_values
                    .get(key)
                    .cloned()
                    .filter(|value| !value.trim().is_empty())
            })
    }

    fn api_key_candidates(&self) -> &'static [&'static str] {
        match &self.provider {
            SearchProvider::Brave => &["BRAVE_API_KEY"],
            SearchProvider::Perplexity { .. } => &["PERPLEXITY_API_KEY", "OPENROUTER_API_KEY"],
            #[cfg(feature = "firecrawl")]
            SearchProvider::Firecrawl { .. } => &["FIRECRAWL_API_KEY"],
        }
    }

    #[cfg(test)]
    async fn env_value_with_provider(&self, key: &str) -> Option<String> {
        let runtime_env = self.runtime_env_values().await;
        Self::lookup_env_value(&runtime_env, key)
    }

    async fn current_api_key(&self) -> String {
        let configured = self.configured_api_key.expose_secret();
        if !configured.trim().is_empty() {
            return configured.clone();
        }

        let runtime_env = self.runtime_env_values().await;
        for key in self.api_key_candidates() {
            if let Some(value) = Self::lookup_env_value(&runtime_env, key) {
                return value;
            }
        }
        String::new()
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

    fn cache_key(
        &self,
        key_state: &str,
        query: &str,
        count: u8,
        brave_params: &brave::Params,
        accept_language: Option<&str>,
    ) -> String {
        let provider_context = match &self.provider {
            SearchProvider::Brave => format!(":{brave_params:?}:{accept_language:?}"),
            _ => String::new(),
        };
        format!(
            "{:?}:{key_state}:{query:?}:{count}{provider_context}",
            self.provider
        )
    }

    async fn search_brave(
        &self,
        query: &str,
        count: u8,
        params: &brave::Params,
        accept_language: Option<&str>,
        api_key: &str,
    ) -> crate::Result<serde_json::Value> {
        if api_key.trim().is_empty() {
            return Ok(serde_json::json!({
                "error": "Brave Search API key not configured",
                "hint": "Set BRAVE_API_KEY environment variable or tools.web.search.api_key in config"
            }));
        }

        let url = params.request_url(self.brave_endpoint(), query, count);
        let mut resp = self
            .send_brave_request(&url, accept_language, api_key)
            .await?;

        if resp.status() == reqwest::StatusCode::UNPROCESSABLE_ENTITY
            && params.has_optional_filters()
        {
            debug!(
                "Brave rejected optional search parameters; retrying without localization or freshness"
            );
            let retry_url =
                brave::Params::default().request_url(self.brave_endpoint(), query, count);
            resp = self
                .send_brave_request(&retry_url, accept_language, api_key)
                .await?;
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::message(format!(
                "Brave Search API returned {status}: {body}"
            )));
        }

        let body_text = resp.text().await.map_err(|error| {
            Error::message(format!("failed to read Brave response body: {error}"))
        })?;
        let body: serde_json::Value = serde_json::from_str(&body_text).map_err(|error| {
            let snippet: String = body_text.chars().take(400).collect();
            Error::message(format!(
                "failed to parse Brave JSON body: {error}; body starts with: {snippet}"
            ))
        })?;
        let results = parse_brave_results(&body);

        Ok(serde_json::json!({
            "provider": "brave",
            "query": query,
            "results": results,
        }))
    }

    async fn send_brave_request(
        &self,
        url: &str,
        accept_language: Option<&str>,
        api_key: &str,
    ) -> crate::Result<reqwest::Response> {
        let client = crate::shared_http_client();
        let mut request = client
            .get(url)
            .timeout(self.timeout)
            .header("Accept", "application/json")
            .header("X-Subscription-Token", api_key);
        if let Some(lang) = accept_language {
            request = request.header("Accept-Language", lang);
        }
        Ok(request.send().await?)
    }

    async fn search_perplexity(
        &self,
        query: &str,
        api_key: &str,
        base_url: &str,
        model: &str,
    ) -> crate::Result<serde_json::Value> {
        if api_key.trim().is_empty() {
            return Ok(serde_json::json!({
                "error": "Perplexity API key not configured",
                "hint": "Set PERPLEXITY_API_KEY or OPENROUTER_API_KEY environment variable, or tools.web.search.perplexity.api_key in config"
            }));
        }

        let client = crate::shared_http_client();

        let body = serde_json::json!({
            "model": model,
            "messages": [
                {"role": "user", "content": query}
            ]
        });

        let resp = client
            .post(format!("{base_url}/chat/completions"))
            .timeout(self.timeout)
            .header("Authorization", format!("Bearer {api_key}"))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::message(format!(
                "Perplexity API returned {status}: {text}"
            )));
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

    /// Check whether DuckDuckGo is temporarily blocked due to a prior CAPTCHA.
    fn is_ddg_blocked(&self) -> bool {
        self.ddg_blocked_until
            .lock()
            .ok()
            .and_then(|guard| *guard)
            .is_some_and(|until| Instant::now() < until)
    }

    /// Block DuckDuckGo for `duration` after a CAPTCHA response.
    fn block_ddg(&self, duration: Duration) {
        if let Ok(mut guard) = self.ddg_blocked_until.lock() {
            *guard = Some(Instant::now() + duration);
        }
    }

    /// Fallback: search DuckDuckGo's HTML endpoint when no API key is configured.
    async fn search_duckduckgo(&self, query: &str, count: u8) -> crate::Result<serde_json::Value> {
        // Fail fast if DDG recently returned a CAPTCHA.
        if self.is_ddg_blocked() {
            return Err(Error::message(
                "Web search unavailable: DuckDuckGo is rate-limited (CAPTCHA) and no search \
                 API key is configured. Set BRAVE_API_KEY or PERPLEXITY_API_KEY to enable search.",
            ));
        }

        let client = crate::shared_http_client();

        let resp = client
            .post("https://html.duckduckgo.com/html/")
            .timeout(self.timeout)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Referer", "https://html.duckduckgo.com/")
            .body(format!("q={}&b=", urlencoding::encode(query)))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(Error::message(format!("DuckDuckGo returned HTTP {status}")));
        }

        let html = resp.text().await?;

        if html.contains("challenge-form") || html.contains("not a Robot") {
            // Block DDG for 1 hour so subsequent calls fail instantly.
            self.block_ddg(Duration::from_secs(3600));
            warn!("DuckDuckGo CAPTCHA detected — blocking fallback for 1 hour");
            return Err(Error::message(
                "Web search unavailable: DuckDuckGo returned a CAPTCHA challenge. \
                 Configure BRAVE_API_KEY or PERPLEXITY_API_KEY for reliable search.",
            ));
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

fn resolve_perplexity_base_url(base_url_override: Option<&str>, api_key: &str) -> String {
    if let Some(base_url) = base_url_override {
        return base_url.to_string();
    }
    if api_key.starts_with("pplx-") {
        "https://api.perplexity.ai".to_string()
    } else {
        // Assume OpenRouter for sk-or- or other keys.
        "https://openrouter.ai/api/v1".to_string()
    }
}

// ---------------------------------------------------------------------------
// DuckDuckGo HTML parsing helpers
// ---------------------------------------------------------------------------

/// Parse Brave JSON response into normalized result rows.
fn parse_brave_results(body: &serde_json::Value) -> Vec<BraveResult> {
    body.get("web")
        .and_then(|web| web.get("results"))
        .and_then(serde_json::Value::as_array)
        .map(|results| {
            results
                .iter()
                .filter_map(|result| {
                    let title = result
                        .get("title")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .unwrap_or("");
                    let url = result
                        .get("url")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .unwrap_or("");
                    if title.is_empty() || url.is_empty() {
                        return None;
                    }
                    let description = result
                        .get("description")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string();

                    Some(BraveResult {
                        title: title.to_string(),
                        url: url.to_string(),
                        description,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

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
        let mut properties = serde_json::Map::from_iter([
            (
                "query".to_string(),
                serde_json::json!({
                    "type": "string",
                    "description": "The search query"
                }),
            ),
            (
                "count".to_string(),
                serde_json::json!({
                    "type": "integer",
                    "description": "Number of results (1-10, default 5)",
                    "minimum": 1,
                    "maximum": 10
                }),
            ),
        ]);

        if matches!(&self.provider, SearchProvider::Brave) {
            properties.extend(brave::parameter_properties());
        }

        serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": ["query"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing 'query' parameter"))?;

        let count = params
            .get("count")
            .and_then(|v| v.as_u64())
            .map(|n| n.clamp(1, 10) as u8)
            .unwrap_or(self.max_results);

        let api_key = self.current_api_key().await;

        // Include key presence in cache key so hot key changes invalidate stale
        // no-key results.
        let key_state = if api_key.is_empty() {
            "no-key"
        } else {
            "has-key"
        };
        let accept_language = params.get("_accept_language").and_then(|v| v.as_str());
        let brave_params = brave::Params::from_json(&params);
        let cache_key = self.cache_key(key_state, query, count, &brave_params, accept_language);
        if let Some(cached) = self.cache_get(&cache_key) {
            debug!("web_search cache hit for: {query}");
            return Ok(cached);
        }

        debug!("web_search: {query} (count={count})");

        // When no API key is configured, skip the provider entirely and go
        // straight to the DuckDuckGo fallback. This avoids a pointless
        // round-trip that always returns an error and prevents the LLM from
        // retrying repeatedly.
        let result = if self.fallback_enabled && api_key.is_empty() {
            warn!(
                provider = ?self.provider,
                "search API key not configured, using DuckDuckGo directly"
            );
            self.search_duckduckgo(query, count).await?
        } else {
            match &self.provider {
                SearchProvider::Brave => {
                    self.search_brave(query, count, &brave_params, accept_language, &api_key)
                        .await?
                },
                SearchProvider::Perplexity {
                    base_url_override,
                    model,
                } => {
                    let base_url =
                        resolve_perplexity_base_url(base_url_override.as_deref(), &api_key);
                    self.search_perplexity(query, &api_key, &base_url, model)
                        .await?
                },
                #[cfg(feature = "firecrawl")]
                SearchProvider::Firecrawl { base_url } => {
                    if api_key.is_empty() {
                        serde_json::json!({
                            "error": "Firecrawl API key not configured",
                            "hint": "Set FIRECRAWL_API_KEY environment variable or tools.web.firecrawl.api_key in config"
                        })
                    } else {
                        crate::firecrawl::firecrawl_search(
                            crate::shared_http_client(),
                            base_url,
                            &api_key,
                            query,
                            count,
                            self.timeout,
                        )
                        .await?
                    }
                },
            }
        };

        self.cache_set(cache_key, result.clone());
        Ok(result)
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, std::sync::Arc};

    struct MockEnvProvider {
        vars: Vec<(String, String)>,
    }

    #[async_trait]
    impl EnvVarProvider for MockEnvProvider {
        async fn get_env_vars(&self) -> Vec<(String, Secret<String>)> {
            self.vars
                .iter()
                .map(|(k, v)| (k.clone(), Secret::new(v.clone())))
                .collect()
        }
    }

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
                base_url_override: Some("https://api.perplexity.ai".into()),
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
        assert!(schema["properties"]["country"]["enum"].is_array());
        assert!(schema["properties"]["search_lang"]["enum"].is_array());
        assert!(schema["properties"]["ui_lang"]["enum"].is_array());
    }

    #[test]
    fn test_non_brave_schema_omits_brave_parameters() {
        let schema = perplexity_tool().parameters_schema();
        let properties = schema["properties"].as_object().expect("properties");
        assert!(!properties.contains_key("country"));
        assert!(!properties.contains_key("search_lang"));
        assert!(!properties.contains_key("ui_lang"));
        assert!(!properties.contains_key("freshness"));
    }

    #[test]
    fn test_brave_cache_key_uses_normalized_localization() {
        let tool = brave_tool();
        let normalized = brave::Params::from_json(&serde_json::json!({
            "country": "BR",
            "search_lang": "pt-br",
            "ui_lang": "pt-BR",
            "freshness": "pw",
        }));
        let aliases = brave::Params::from_json(&serde_json::json!({
            "country": "br",
            "search_lang": "pt",
            "ui_lang": "pt_BR",
            "freshness": "week",
        }));
        let other_market = brave::Params::from_json(&serde_json::json!({
            "country": "US",
            "search_lang": "en",
            "ui_lang": "en-US",
            "freshness": "pw",
        }));

        let key = tool.cache_key("has-key", "query", 5, &normalized, Some("pt-BR"));
        assert_eq!(
            key,
            tool.cache_key("has-key", "query", 5, &aliases, Some("pt-BR"))
        );
        assert_ne!(
            key,
            tool.cache_key("has-key", "query", 5, &other_market, Some("pt-BR"))
        );
        assert_ne!(
            key,
            tool.cache_key("has-key", "query", 5, &normalized, Some("en-US"))
        );
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
        let params = brave::Params::default();
        let result = tool
            .search_brave("test", 5, &params, None, "")
            .await
            .unwrap();
        assert!(result["error"].as_str().unwrap().contains("not configured"));
        assert!(result["hint"].as_str().unwrap().contains("BRAVE_API_KEY"));
    }

    #[tokio::test]
    async fn test_brave_retries_422_without_optional_filters() {
        let mut server = mockito::Server::new_async().await;
        let first_request = server
            .mock("GET", "/res/v1/web/search")
            .match_query(mockito::Matcher::Exact(
                "q=edge%20query&count=5&country=ALL&search_lang=es&freshness=pw".into(),
            ))
            .match_header("x-subscription-token", "test-key")
            .match_header("accept-language", "pt-BR")
            .with_status(422)
            .expect(1)
            .create_async()
            .await;
        let retry_request = server
            .mock("GET", "/res/v1/web/search")
            .match_query(mockito::Matcher::Exact("q=edge%20query&count=5".into()))
            .match_header("x-subscription-token", "test-key")
            .match_header("accept-language", "pt-BR")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"web":{"results":[{"title":"Recovered","url":"https://example.com","description":"Retry result"}]}}"#,
            )
            .expect(1)
            .create_async()
            .await;
        let tool = brave_tool().with_brave_endpoint(format!("{}/res/v1/web/search", server.url()));

        let params = brave::Params::from_json(&serde_json::json!({
            "country": "PY",
            "search_lang": "es-AR",
            "ui_lang": "es-PY",
            "freshness": "week",
        }));

        let result = tool
            .search_brave("edge query", 5, &params, Some("pt-BR"), "test-key")
            .await
            .unwrap();

        assert_eq!(result["provider"], "brave");
        assert_eq!(result["query"], "edge query");
        assert_eq!(result["results"][0]["title"], "Recovered");
        first_request.assert_async().await;
        retry_request.assert_async().await;
    }

    #[tokio::test]
    async fn test_brave_does_not_retry_422_without_optional_filters() {
        let mut server = mockito::Server::new_async().await;
        let request = server
            .mock("GET", "/res/v1/web/search")
            .match_query(mockito::Matcher::Exact("q=plain&count=5".into()))
            .with_status(422)
            .with_body("invalid query")
            .expect(1)
            .create_async()
            .await;
        let tool = brave_tool().with_brave_endpoint(format!("{}/res/v1/web/search", server.url()));

        let params = brave::Params::default();
        let error = tool
            .search_brave("plain", 5, &params, None, "test-key")
            .await
            .unwrap_err();

        assert!(error.to_string().contains("422 Unprocessable Entity"));
        assert!(error.to_string().contains("invalid query"));
        request.assert_async().await;
    }

    #[tokio::test]
    async fn test_brave_surfaces_retry_response_error() {
        let mut server = mockito::Server::new_async().await;
        let first_request = server
            .mock("GET", "/res/v1/web/search")
            .match_query(mockito::Matcher::Exact(
                "q=unavailable&count=5&country=US".into(),
            ))
            .with_status(422)
            .expect(1)
            .create_async()
            .await;
        let retry_request = server
            .mock("GET", "/res/v1/web/search")
            .match_query(mockito::Matcher::Exact("q=unavailable&count=5".into()))
            .with_status(503)
            .with_body("temporary outage")
            .expect(1)
            .create_async()
            .await;
        let tool = brave_tool().with_brave_endpoint(format!("{}/res/v1/web/search", server.url()));

        let params = brave::Params::from_json(&serde_json::json!({"country": "US"}));

        let error = tool
            .search_brave("unavailable", 5, &params, None, "test-key")
            .await
            .unwrap_err();

        assert!(error.to_string().contains("503 Service Unavailable"));
        assert!(error.to_string().contains("temporary outage"));
        first_request.assert_async().await;
        retry_request.assert_async().await;
    }

    #[tokio::test]
    async fn test_perplexity_missing_api_key_returns_hint() {
        let tool = perplexity_tool();
        let result = tool
            .search_perplexity("test", "", "https://api.perplexity.ai", "sonar-pro")
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
        let results = parse_brave_results(&json);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Rust");
    }

    #[test]
    fn test_brave_response_parsing_tolerates_nulls() {
        let json = serde_json::json!({
            "web": {
                "results": [
                    {"title": "Valid", "url": "https://example.com", "description": null},
                    {"title": null, "url": "https://invalid.example", "description": "ignored"},
                    {"title": "MissingUrl", "url": null, "description": "ignored"}
                ]
            }
        });
        let results = parse_brave_results(&json);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Valid");
        assert_eq!(results[0].url, "https://example.com");
        assert_eq!(results[0].description, "");
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
    fn test_resolve_perplexity_base_url_pplx_prefix() {
        let url = resolve_perplexity_base_url(None, "pplx-abc123");
        assert!(url.contains("perplexity.ai"));
    }

    #[test]
    fn test_resolve_perplexity_base_url_openrouter_prefix() {
        let url = resolve_perplexity_base_url(None, "sk-or-abc123");
        assert!(url.contains("openrouter.ai"));
    }

    #[test]
    fn test_resolve_perplexity_base_url_respects_override() {
        let url = resolve_perplexity_base_url(Some("https://custom.example"), "pplx-abc123");
        assert_eq!(url, "https://custom.example");
    }

    #[tokio::test]
    async fn test_env_value_with_provider_uses_runtime_env_values() {
        let key = format!("MOLTIS_TEST_DYNAMIC_KEY_{}", std::process::id());
        let provider = Arc::new(MockEnvProvider {
            vars: vec![(key.clone(), "dynamic-value".to_string())],
        });
        let tool = brave_tool().with_env_provider(provider);

        assert_eq!(
            tool.env_value_with_provider(&key).await.as_deref(),
            Some("dynamic-value")
        );
    }

    #[tokio::test]
    async fn test_current_api_key_prefers_configured_value() {
        let provider = Arc::new(MockEnvProvider {
            vars: vec![("BRAVE_API_KEY".to_string(), "runtime-key".to_string())],
        });
        let tool = WebSearchTool::new(
            SearchProvider::Brave,
            Secret::new("configured-key".to_string()),
            5,
            Duration::from_secs(10),
            Duration::from_secs(60),
            false,
        )
        .with_env_provider(provider);

        assert_eq!(tool.current_api_key().await, "configured-key");
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
    fn test_from_config_none_without_key_or_fallback() {
        let cfg = WebSearchConfig::default();
        assert!(
            WebSearchTool::from_config(&cfg).is_none(),
            "tool should not register without an API key and no DDG fallback"
        );
    }

    #[test]
    fn test_from_config_can_enable_ddg_fallback() {
        let cfg = WebSearchConfig {
            duckduckgo_fallback: true,
            ..Default::default()
        };
        let tool = WebSearchTool::from_config(&cfg).expect("web search should be enabled");
        assert!(tool.fallback_enabled);
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
    fn test_ddg_cooldown_blocks_after_captcha() {
        let tool = brave_tool();
        // Initially not blocked.
        assert!(!tool.is_ddg_blocked());
        // Simulate a CAPTCHA cooldown.
        tool.block_ddg(Duration::from_secs(3600));
        assert!(tool.is_ddg_blocked());
    }

    #[test]
    fn test_ddg_cooldown_expires() {
        let tool = brave_tool();
        // Block for zero seconds — should expire immediately.
        tool.block_ddg(Duration::ZERO);
        assert!(!tool.is_ddg_blocked());
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
