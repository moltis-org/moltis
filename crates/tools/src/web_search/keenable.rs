//! Keenable web search provider.
//!
//! Keenable answers without an API key on its public endpoint. An API key only
//! lifts the rate limits and moves requests to the keyed endpoint.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::Error;

pub(super) const BASE_URL: &str = "https://api.keenable.ai";

/// App label Keenable requires on keyless requests.
const APP_TITLE: &str = "Moltis";

/// Keenable snippets are raw page text, often several KB. Keep each result
/// close to the size of the descriptions other providers return.
const MAX_SNIPPET_CHARS: usize = 1_000;

#[derive(Serialize)]
struct SearchRequest<'a> {
    query: &'a str,
    max_results: u8,
}

#[derive(Debug, Default, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<RawResult>,
}

#[derive(Debug, Default, Deserialize)]
struct RawResult {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    snippet: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct SearchResult {
    title: String,
    url: String,
    description: String,
}

pub(super) async fn search(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    query: &str,
    count: u8,
    timeout: Duration,
) -> crate::Result<serde_json::Value> {
    let keyless = api_key.trim().is_empty();
    let path = if keyless {
        "/v1/search/public"
    } else {
        "/v1/search"
    };
    let mut request = client
        .post(format!("{}{path}", base_url.trim_end_matches('/')))
        .timeout(timeout)
        .header("Accept", "application/json")
        .header("X-Keenable-Title", APP_TITLE)
        .json(&SearchRequest {
            query,
            max_results: count,
        });
    if !keyless {
        request = request.header("X-API-Key", api_key);
    }
    let resp = request.send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let hint = if keyless && status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            " (keyless rate limit reached; set KEENABLE_API_KEY for higher limits)"
        } else {
            ""
        };
        return Err(Error::message(format!(
            "Keenable Search API returned {status}: {body}{hint}"
        )));
    }

    let body: SearchResponse = resp.json().await?;

    Ok(serde_json::json!({
        "provider": "keenable",
        "query": query,
        "results": parse_results(body),
    }))
}

fn parse_results(body: SearchResponse) -> Vec<SearchResult> {
    body.results
        .into_iter()
        .filter_map(|result| {
            let url = result.url.as_deref().map(str::trim).unwrap_or("");
            if url.is_empty() {
                return None;
            }
            let title = result
                .title
                .as_deref()
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .unwrap_or(url);
            // `snippet` carries the result text; `description` is kept as a
            // fallback in case the API fills it.
            let text = result
                .snippet
                .as_deref()
                .filter(|snippet| !snippet.trim().is_empty())
                .or(result.description.as_deref())
                .unwrap_or("");

            Some(SearchResult {
                title: title.to_string(),
                url: url.to_string(),
                description: compact_snippet(text),
            })
        })
        .collect()
}

/// Collapse whitespace runs (snippets keep the page's line breaks) and cap the
/// length at a char boundary.
fn compact_snippet(text: &str) -> String {
    const ELLIPSIS: &str = "...";
    let mut out = String::new();
    let mut char_count = 0usize;
    let mut first = true;
    let mut truncated = false;

    for word in text.split_whitespace() {
        if !first {
            if char_count >= MAX_SNIPPET_CHARS {
                truncated = true;
                break;
            }
            if char_count + 1 > MAX_SNIPPET_CHARS {
                truncated = true;
                break;
            }
            out.push(' ');
            char_count += 1;
        }
        first = false;

        let word_len = word.chars().count();
        if char_count + word_len <= MAX_SNIPPET_CHARS {
            out.push_str(word);
            char_count += word_len;
        } else {
            let room = MAX_SNIPPET_CHARS - char_count;
            out.extend(word.chars().take(room));
            truncated = true;
            break;
        }
    }

    if truncated {
        out.push_str(ELLIPSIS);
    }
    out
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::web_search::{ConfigSearchProvider, SearchProvider, WebSearchConfig, WebSearchTool},
        mockito::Matcher,
        moltis_agents::tool_registry::AgentTool,
        secrecy::Secret,
    };

    fn response_body() -> String {
        serde_json::json!({
            "results": [
                {
                    "title": "Rust",
                    "url": "https://www.rust-lang.org/",
                    "description": "",
                    "snippet": "Rust\n\nA language empowering everyone\n  to build reliable software.",
                    "acquired_at": "2026-10-30T12:00:00Z"
                },
                {
                    "title": "Tokio",
                    "url": "https://tokio.rs/",
                    "description": "",
                    "snippet": "An asynchronous runtime."
                }
            ]
        })
        .to_string()
    }

    #[tokio::test]
    async fn keyless_search_uses_public_endpoint_with_app_title() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/search/public")
            .match_header("x-keenable-title", "Moltis")
            .match_header("x-api-key", Matcher::Missing)
            .match_body(Matcher::Json(serde_json::json!({
                "query": "rust async",
                "max_results": 3
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(response_body())
            .expect(1)
            .create_async()
            .await;

        let result = search(
            &reqwest::Client::new(),
            &server.url(),
            "",
            "rust async",
            3,
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        assert_eq!(result["provider"], "keenable");
        assert_eq!(result["query"], "rust async");
        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["title"], "Rust");
        assert_eq!(results[0]["url"], "https://www.rust-lang.org/");
        assert_eq!(
            results[0]["description"],
            "Rust A language empowering everyone to build reliable software."
        );
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn keyed_search_uses_key_endpoint() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/search")
            .match_header("x-keenable-title", "Moltis")
            .match_header("x-api-key", "test-key")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(response_body())
            .expect(1)
            .create_async()
            .await;

        let result = search(
            &reqwest::Client::new(),
            &format!("{}/", server.url()),
            "test-key",
            "rust",
            5,
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        assert_eq!(result["results"].as_array().unwrap().len(), 2);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn keyless_rate_limit_error_suggests_api_key() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/search/public")
            .with_status(429)
            .with_body("rate limited")
            .expect(1)
            .create_async()
            .await;

        let error = search(
            &reqwest::Client::new(),
            &server.url(),
            "",
            "rust",
            5,
            Duration::from_secs(10),
        )
        .await
        .unwrap_err();

        let message = error.to_string();
        assert!(message.contains("429 Too Many Requests"));
        assert!(message.contains("rate limited"));
        assert!(message.contains("KEENABLE_API_KEY"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn keyed_error_has_no_key_hint() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/search")
            .with_status(401)
            .with_body("invalid key")
            .expect(1)
            .create_async()
            .await;

        let error = search(
            &reqwest::Client::new(),
            &server.url(),
            "bad-key",
            "rust",
            5,
            Duration::from_secs(10),
        )
        .await
        .unwrap_err();

        let message = error.to_string();
        assert!(message.contains("401 Unauthorized"));
        assert!(!message.contains("KEENABLE_API_KEY"));
        mock.assert_async().await;
    }

    #[test]
    fn from_config_registers_without_key() {
        let cfg = WebSearchConfig {
            provider: ConfigSearchProvider::Keenable,
            ..Default::default()
        };
        let tool = WebSearchTool::from_config(&cfg).expect("Keenable should not need a key");
        assert!(matches!(tool.provider, SearchProvider::Keenable { .. }));
        assert!(!tool.fallback_enabled);
    }

    #[tokio::test]
    async fn tool_without_key_calls_keenable_instead_of_duckduckgo() {
        let mut server = mockito::Server::new_async().await;
        let request = server
            .mock("POST", "/v1/search/public")
            .match_header("x-keenable-title", "Moltis")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(response_body())
            .expect(1)
            .create_async()
            .await;
        let tool = WebSearchTool::new(
            SearchProvider::Keenable {
                base_url: server.url(),
            },
            Secret::new(String::new()),
            5,
            Duration::from_secs(10),
            Duration::ZERO,
            true, // DuckDuckGo fallback on: Keenable is still called without a key
        );

        let result = tool
            .execute(serde_json::json!({"query": "rust"}))
            .await
            .unwrap();

        assert_eq!(result["provider"], "keenable");
        assert_eq!(
            result["results"][1]["description"],
            "An asynchronous runtime."
        );
        request.assert_async().await;
    }

    #[test]
    fn parse_results_skips_rows_without_url_and_falls_back_to_description() {
        let body: SearchResponse = serde_json::from_value(serde_json::json!({
            "results": [
                {"title": "No URL", "url": null, "snippet": "ignored"},
                {"title": "", "url": " https://example.com/a ", "snippet": "", "description": "From description"},
                {"title": "Null text", "url": "https://example.com/b", "snippet": null, "description": null}
            ]
        }))
        .unwrap();

        assert_eq!(parse_results(body), vec![
            SearchResult {
                title: "https://example.com/a".into(),
                url: "https://example.com/a".into(),
                description: "From description".into(),
            },
            SearchResult {
                title: "Null text".into(),
                url: "https://example.com/b".into(),
                description: String::new(),
            },
        ]);
    }

    #[test]
    fn parse_results_tolerates_missing_results() {
        let body: SearchResponse = serde_json::from_str("{}").unwrap();
        assert!(parse_results(body).is_empty());
    }

    #[test]
    fn compact_snippet_caps_length_at_char_boundary() {
        let long = "é".repeat(MAX_SNIPPET_CHARS + 5);
        let compact = compact_snippet(&long);
        assert_eq!(compact.chars().count(), MAX_SNIPPET_CHARS + 3);
        assert!(compact.ends_with("..."));

        let exact = "a".repeat(MAX_SNIPPET_CHARS);
        assert_eq!(compact_snippet(&exact), exact);
    }
}
