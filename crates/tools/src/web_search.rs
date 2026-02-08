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
    tracing::debug,
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
pub struct WebSearchTool {
    provider: SearchProvider,
    api_key: Secret<String>,
    max_results: u8,
    timeout: Duration,
    cache_ttl: Duration,
    cache: Mutex<HashMap<String, CacheEntry>>,
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
    ) -> Self {
        Self {
            provider,
            api_key,
            max_results,
            timeout,
            cache_ttl,
            cache: Mutex::new(HashMap::new()),
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
}
