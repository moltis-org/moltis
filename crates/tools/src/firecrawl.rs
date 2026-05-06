//! Firecrawl integration — standalone scrape tool and shared helpers.
//!
//! Firecrawl is a hosted web scraping/search API that returns clean markdown
//! from web pages, including JS-heavy and bot-protected sites.
//!
//! This module provides:
//! - `FirecrawlScrapeTool` — an `AgentTool` for direct `firecrawl_scrape` calls
//! - `firecrawl_scrape()` — a helper used by both the tool and the `web_fetch` fallback

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use {
    async_trait::async_trait,
    secrecy::{ExposeSecret, Secret},
    tracing::debug,
};

use {
    crate::error::Error, moltis_agents::tool_registry::AgentTool,
    moltis_config::schema::FirecrawlConfig,
};

/// Default Firecrawl API base URL.
pub(crate) const DEFAULT_BASE_URL: &str = "https://api.firecrawl.dev";

/// Default maximum characters returned from a scrape.
const DEFAULT_MAX_CHARS: usize = 50_000;

/// Cached scrape result with expiry.
struct CacheEntry {
    value: serde_json::Value,
    expires_at: Instant,
}

/// Firecrawl scrape tool — scrape a URL via the Firecrawl API and return
/// clean markdown.  Useful for JS-heavy or bot-protected pages where plain
/// `web_fetch` extraction is weak.
pub struct FirecrawlScrapeTool {
    api_key: Secret<String>,
    base_url: String,
    only_main_content: bool,
    timeout: Duration,
    cache_ttl: Duration,
    cache: Mutex<HashMap<String, CacheEntry>>,
}

/// Result of a Firecrawl scrape operation.
#[derive(Debug)]
pub struct ScrapeResult {
    pub markdown: String,
    pub title: Option<String>,
    pub source_url: Option<String>,
    pub status_code: Option<u16>,
}

impl FirecrawlScrapeTool {
    /// Build from config; returns `None` if disabled or no API key available.
    pub fn from_config(config: &FirecrawlConfig) -> Option<Self> {
        if !config.enabled {
            return None;
        }
        let api_key = resolve_api_key(config)?;
        Some(Self {
            api_key,
            base_url: if config.base_url.trim().is_empty() {
                DEFAULT_BASE_URL.into()
            } else {
                config.base_url.clone()
            },
            only_main_content: config.only_main_content,
            timeout: Duration::from_secs(config.timeout_seconds),
            cache_ttl: Duration::from_secs(config.cache_ttl_minutes * 60),
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
}

#[async_trait]
impl AgentTool for FirecrawlScrapeTool {
    fn name(&self) -> &str {
        "firecrawl_scrape"
    }

    fn description(&self) -> &str {
        "Scrape a web page using Firecrawl and return clean markdown content. \
         Useful for JavaScript-heavy or bot-protected pages where plain web_fetch \
         produces poor results. Requires a Firecrawl API key."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The HTTP or HTTPS URL to scrape via Firecrawl"
                },
                "extract_mode": {
                    "type": "string",
                    "enum": ["markdown", "text"],
                    "description": "Content extraction mode (default: markdown)"
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Maximum characters to return (default: 50000)",
                    "minimum": 100
                },
                "only_main_content": {
                    "type": "boolean",
                    "description": "Extract only the main content, skipping navs/footers (default: true)"
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
            .unwrap_or(DEFAULT_MAX_CHARS);

        let only_main_content = params
            .get("only_main_content")
            .and_then(|v| v.as_bool())
            .unwrap_or(self.only_main_content);

        // Cache lookup.
        let cache_key = format!("firecrawl:{url}:{extract_mode}:{max_chars}:{only_main_content}");
        if let Some(cached) = self.cache_get(&cache_key) {
            debug!("firecrawl_scrape cache hit for: {url}");
            return Ok(cached);
        }

        debug!("firecrawl_scrape: {url}");
        let scrape = firecrawl_scrape(
            crate::shared_http_client(),
            &self.base_url,
            self.api_key.expose_secret(),
            url,
            only_main_content,
            self.timeout,
        )
        .await?;

        let Some(scrape) = scrape else {
            return Ok(serde_json::json!({
                "error": "Firecrawl returned no content for this URL",
                "url": url,
            }));
        };

        let content = if extract_mode == "text" {
            // Simple markdown-to-text: strip common markdown syntax.
            strip_markdown(&scrape.markdown)
        } else {
            scrape.markdown.clone()
        };

        let truncated = content.len() > max_chars;
        let content = if truncated {
            truncate_at_char_boundary(&content, max_chars)
        } else {
            content
        };

        let result = serde_json::json!({
            "url": url,
            "final_url": scrape.source_url.as_deref().unwrap_or(url),
            "title": scrape.title,
            "status": scrape.status_code,
            "extractor": "firecrawl",
            "extract_mode": extract_mode,
            "content": content,
            "truncated": truncated,
        });

        self.cache_set(cache_key, result.clone());
        Ok(result)
    }
}

/// Scrape a URL via the Firecrawl `/v1/scrape` API.
///
/// Returns `Ok(Some(result))` on success, `Ok(None)` when Firecrawl returns
/// no content, and `Err` on network/API failures.
///
/// Used by both `FirecrawlScrapeTool` and the `web_fetch` fallback path.
pub async fn firecrawl_scrape(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    url: &str,
    only_main_content: bool,
    timeout: Duration,
) -> crate::Result<Option<ScrapeResult>> {
    let endpoint = format!("{}/v1/scrape", base_url.trim_end_matches('/'));

    // Send a shorter timeout to Firecrawl so it can return a structured
    // error before the HTTP client tears down the connection.
    let api_timeout_ms = timeout.as_millis().saturating_sub(5_000).max(5_000) as u64;
    let body = serde_json::json!({
        "url": url,
        "formats": ["markdown"],
        "onlyMainContent": only_main_content,
        "timeout": api_timeout_ms,
    });

    let resp = client
        .post(&endpoint)
        .timeout(timeout)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::message(format!("Firecrawl request failed: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        let error_body = resp.text().await.unwrap_or_default();
        return Err(Error::message(format!(
            "Firecrawl API error (HTTP {status}): {error_body}"
        )));
    }

    let payload: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| Error::message(format!("Firecrawl response parse failed: {e}")))?;

    // Check API-level success flag.
    if payload.get("success").and_then(|v| v.as_bool()) == Some(false) {
        let error = payload
            .get("error")
            .or_else(|| payload.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(Error::message(format!("Firecrawl API error: {error}")));
    }

    let data = match payload.get("data") {
        Some(d) if d.is_object() => d,
        _ => return Ok(None),
    };

    let markdown = data
        .get("markdown")
        .or_else(|| data.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if markdown.is_empty() {
        return Ok(None);
    }

    let metadata = data.get("metadata");

    let title = metadata
        .and_then(|m| m.get("title"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let source_url = metadata
        .and_then(|m| m.get("sourceURL"))
        .or_else(|| data.get("url"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let status_code = metadata
        .and_then(|m| m.get("statusCode"))
        .or_else(|| data.get("statusCode"))
        .and_then(|v| v.as_u64())
        .map(|n| n as u16);

    Ok(Some(ScrapeResult {
        markdown,
        title,
        source_url,
        status_code,
    }))
}

/// Resolve the Firecrawl API key from config or environment.
pub fn resolve_api_key(config: &FirecrawlConfig) -> Option<Secret<String>> {
    if let Some(ref key) = config.api_key {
        let value = key.expose_secret();
        if !value.trim().is_empty() {
            return Some(key.clone());
        }
    }
    std::env::var("FIRECRAWL_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(Secret::new)
}

/// Search the web via the Firecrawl `/v1/search` API.
///
/// Returns a JSON value matching the format used by `WebSearchTool`.
pub async fn firecrawl_search(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    query: &str,
    count: u8,
    timeout: Duration,
) -> crate::Result<serde_json::Value> {
    let endpoint = format!("{}/v1/search", base_url.trim_end_matches('/'));

    let body = serde_json::json!({
        "query": query,
        "limit": count,
    });

    let resp = client
        .post(&endpoint)
        .timeout(timeout)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::message(format!("Firecrawl search request failed: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        let error_body = resp.text().await.unwrap_or_default();
        return Err(Error::message(format!(
            "Firecrawl Search API error (HTTP {status}): {error_body}"
        )));
    }

    let payload: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| Error::message(format!("Firecrawl search response parse failed: {e}")))?;

    if payload.get("success").and_then(|v| v.as_bool()) == Some(false) {
        let error = payload
            .get("error")
            .or_else(|| payload.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(Error::message(format!(
            "Firecrawl Search API error: {error}"
        )));
    }

    let results = extract_search_results(&payload);

    Ok(serde_json::json!({
        "query": query,
        "provider": "firecrawl",
        "results": results,
    }))
}

/// Extract search results from the Firecrawl search API response.
fn extract_search_results(payload: &serde_json::Value) -> Vec<serde_json::Value> {
    // Firecrawl returns results in `data` as an array.
    let items = payload
        .get("data")
        .and_then(|v| v.as_array())
        .or_else(|| payload.get("results").and_then(|v| v.as_array()));

    let Some(items) = items else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|entry| {
            let url = entry
                .get("url")
                .or_else(|| entry.get("metadata").and_then(|m| m.get("sourceURL")))
                .and_then(|v| v.as_str())?;

            let title = entry
                .get("title")
                .or_else(|| entry.get("metadata").and_then(|m| m.get("title")))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let description = entry
                .get("description")
                .or_else(|| entry.get("snippet"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            Some(serde_json::json!({
                "title": title,
                "url": url,
                "description": description,
            }))
        })
        .collect()
}

/// Minimal markdown-to-text stripping.
///
/// Removes heading markers (`#`), bold/italic markers (`**`, `__`, `*`).
/// Preserves underscores in identifiers and URLs (e.g. `snake_case`,
/// `https://example.com/some_path`).
fn strip_markdown(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    for line in md.lines() {
        let trimmed = line.trim_start_matches('#').trim_start();
        // Strip bold/italic markers.  Leave lone underscores intact so
        // URLs and identifiers (snake_case) are not mangled.
        let cleaned: String = trimmed.replace("**", "").replace("__", "").replace('*', "");
        out.push_str(&cleaned);
        out.push('\n');
    }
    out.trim().to_string()
}

/// Truncate a string at a char boundary, not mid-UTF-8.
fn truncate_at_char_boundary(s: &str, max: usize) -> String {
    crate::web_fetch::truncate_at_char_boundary(s, max)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(enabled: bool, api_key: Option<&str>) -> FirecrawlConfig {
        FirecrawlConfig {
            enabled,
            api_key: api_key.map(|k| Secret::new(k.to_string())),
            base_url: DEFAULT_BASE_URL.into(),
            only_main_content: true,
            timeout_seconds: 10,
            cache_ttl_minutes: 5,
            web_fetch_fallback: true,
        }
    }

    #[test]
    fn test_from_config_disabled() {
        assert!(FirecrawlScrapeTool::from_config(&test_config(false, Some("fc-key"))).is_none());
    }

    #[test]
    fn test_from_config_no_api_key() {
        assert!(FirecrawlScrapeTool::from_config(&test_config(true, None)).is_none());
    }

    #[test]
    fn test_from_config_empty_api_key() {
        assert!(FirecrawlScrapeTool::from_config(&test_config(true, Some(""))).is_none());
    }

    #[test]
    fn test_from_config_enabled() {
        assert!(FirecrawlScrapeTool::from_config(&test_config(true, Some("fc-key"))).is_some());
    }

    #[test]
    fn test_tool_name_and_schema() {
        let tool = FirecrawlScrapeTool::from_config(&test_config(true, Some("fc-key"))).unwrap();
        assert_eq!(tool.name(), "firecrawl_scrape");
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"][0], "url");
        assert!(schema["properties"]["url"].is_object());
        assert!(schema["properties"]["extract_mode"].is_object());
        assert!(schema["properties"]["max_chars"].is_object());
    }

    #[tokio::test]
    async fn test_missing_url_param() {
        let tool = FirecrawlScrapeTool::from_config(&test_config(true, Some("fc-key"))).unwrap();
        let result = tool.execute(serde_json::json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("url"));
    }

    #[test]
    fn test_cache_hit_and_miss() {
        let tool = FirecrawlScrapeTool::from_config(&test_config(true, Some("fc-key"))).unwrap();
        let key = "test-key".to_string();
        let val = serde_json::json!({"cached": true});

        assert!(tool.cache_get(&key).is_none());
        tool.cache_set(key.clone(), val.clone());
        assert_eq!(tool.cache_get(&key).unwrap(), val);
    }

    #[test]
    fn test_strip_markdown() {
        let md = "# Title\n**bold** and *italic*\n## Heading 2";
        let text = strip_markdown(md);
        assert!(text.contains("Title"));
        assert!(text.contains("bold and italic"));
        assert!(text.contains("Heading 2"));
        assert!(!text.contains('#'));
        assert!(!text.contains("**"));
    }

    #[test]
    fn test_strip_markdown_preserves_underscores() {
        let md = "Use `snake_case_var` and visit https://api.example.com/some_endpoint";
        let text = strip_markdown(md);
        assert!(
            text.contains("snake_case_var"),
            "underscores in identifiers must be preserved"
        );
        assert!(
            text.contains("some_endpoint"),
            "underscores in URLs must be preserved"
        );
    }

    #[test]
    fn test_truncate_at_char_boundary() {
        let s = "hello world";
        assert_eq!(truncate_at_char_boundary(s, 5), "hello");
        assert_eq!(truncate_at_char_boundary(s, 100), "hello world");

        // UTF-8 boundary safety.
        let s = "héllo";
        let t = truncate_at_char_boundary(s, 2);
        assert!(t.len() <= 2);
        assert!(std::str::from_utf8(t.as_bytes()).is_ok());
    }

    #[test]
    fn test_extract_search_results_standard() {
        let payload = serde_json::json!({
            "success": true,
            "data": [
                {"url": "https://a.com", "title": "A", "description": "Desc A"},
                {"url": "https://b.com", "title": "B", "snippet": "Desc B"},
            ]
        });
        let results = extract_search_results(&payload);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["url"], "https://a.com");
        assert_eq!(results[1]["description"], "Desc B");
    }

    #[test]
    fn test_extract_search_results_with_metadata() {
        let payload = serde_json::json!({
            "success": true,
            "data": [
                {
                    "metadata": {"sourceURL": "https://c.com", "title": "C"},
                    "description": "Desc C"
                }
            ]
        });
        let results = extract_search_results(&payload);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["url"], "https://c.com");
        assert_eq!(results[0]["title"], "C");
    }

    #[test]
    fn test_extract_search_results_empty() {
        let payload = serde_json::json!({"success": true});
        let results = extract_search_results(&payload);
        assert!(results.is_empty());
    }

    #[test]
    fn test_extract_search_results_skips_no_url() {
        let payload = serde_json::json!({
            "success": true,
            "data": [
                {"title": "No URL"},
                {"url": "https://valid.com", "title": "Valid"},
            ]
        });
        let results = extract_search_results(&payload);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["url"], "https://valid.com");
    }

    #[tokio::test]
    async fn test_firecrawl_scrape_success() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/scrape")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "success": true,
                    "data": {
                        "markdown": "# Hello World\nContent here.",
                        "metadata": {
                            "title": "Hello",
                            "sourceURL": "https://example.com",
                            "statusCode": 200
                        }
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let result = firecrawl_scrape(
            &reqwest::Client::new(),
            &server.url(),
            "fc-test-key",
            "https://example.com",
            true,
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        let scrape = result.unwrap();
        assert_eq!(scrape.markdown, "# Hello World\nContent here.");
        assert_eq!(scrape.title.as_deref(), Some("Hello"));
        assert_eq!(scrape.source_url.as_deref(), Some("https://example.com"));
        assert_eq!(scrape.status_code, Some(200));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_firecrawl_scrape_api_error() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/scrape")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "success": false,
                    "error": "Rate limit exceeded"
                })
                .to_string(),
            )
            .create_async()
            .await;

        let result = firecrawl_scrape(
            &reqwest::Client::new(),
            &server.url(),
            "fc-test-key",
            "https://example.com",
            true,
            Duration::from_secs(10),
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Rate limit"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_firecrawl_scrape_no_content() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/scrape")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "success": true,
                    "data": {
                        "markdown": "",
                        "metadata": {}
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let result = firecrawl_scrape(
            &reqwest::Client::new(),
            &server.url(),
            "fc-test-key",
            "https://example.com",
            true,
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        assert!(result.is_none());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_firecrawl_scrape_http_error() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/scrape")
            .with_status(500)
            .with_body("Internal Server Error")
            .create_async()
            .await;

        let result = firecrawl_scrape(
            &reqwest::Client::new(),
            &server.url(),
            "fc-test-key",
            "https://example.com",
            true,
            Duration::from_secs(10),
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("500"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_firecrawl_search_success() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/search")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "success": true,
                    "data": [
                        {
                            "url": "https://example.com",
                            "title": "Example",
                            "description": "An example site"
                        },
                        {
                            "url": "https://test.com",
                            "title": "Test",
                            "description": "A test site"
                        }
                    ]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let result = firecrawl_search(
            &reqwest::Client::new(),
            &server.url(),
            "fc-test-key",
            "test query",
            5,
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        assert_eq!(result["provider"], "firecrawl");
        assert_eq!(result["query"], "test query");
        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["url"], "https://example.com");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_firecrawl_search_api_error() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/search")
            .with_status(403)
            .with_body("Forbidden")
            .create_async()
            .await;

        let result = firecrawl_search(
            &reqwest::Client::new(),
            &server.url(),
            "fc-bad-key",
            "test query",
            5,
            Duration::from_secs(10),
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("403"));
        mock.assert_async().await;
    }

    #[test]
    fn test_resolve_api_key_from_config() {
        let config = test_config(true, Some("fc-from-config"));
        let key = resolve_api_key(&config).unwrap();
        assert_eq!(key.expose_secret(), "fc-from-config");
    }

    #[test]
    fn test_resolve_api_key_none_when_empty() {
        let config = test_config(true, None);
        // Only passes when FIRECRAWL_API_KEY is not set in env.
        if std::env::var("FIRECRAWL_API_KEY").is_err() {
            assert!(resolve_api_key(&config).is_none());
        }
    }
}
