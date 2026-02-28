// Pure functions are used by the WASM Guest impl; allow dead_code on non-wasm targets.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

#[cfg(target_arch = "wasm32")]
wit_bindgen::generate!({
    path: "../../../wit",
    world: "http-tool",
});

#[cfg(target_arch = "wasm32")]
use crate::moltis::tool::{
    outgoing_handler::{self, HttpError, HttpRequest},
    types::{ToolError, ToolValue},
};

use serde_json::{Value, json};

const DEFAULT_TIMEOUT_MS: u32 = 12_000;
const DEFAULT_MAX_RESPONSE_BYTES: u64 = 2_000_000;
const DEFAULT_RESULT_COUNT: u8 = 5;

#[cfg(target_arch = "wasm32")]
struct WebSearchWasm;

#[cfg(target_arch = "wasm32")]
impl Guest for WebSearchWasm {
    fn name() -> String {
        "web_search_wasm".to_string()
    }

    fn description() -> String {
        "Search the web through host HTTP capability (Brave Search API).".to_string()
    }

    fn parameters_schema() -> String {
        json!({
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
                    "description": "Freshness filter: pd, pw, pm, py"
                }
            },
            "required": ["query"]
        })
        .to_string()
    }

    fn execute(params_json: String) -> ToolResult {
        match execute_impl(&params_json) {
            Ok(value) => ToolResult::Ok(ToolValue::Json(value.to_string())),
            Err(error) => ToolResult::Err(error),
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn execute_impl(params_json: &str) -> Result<Value, ToolError> {
    let params: Value = serde_json::from_str(params_json).map_err(|error| ToolError {
        code: "invalid_params_json".to_string(),
        message: error.to_string(),
    })?;
    let query = params
        .get("query")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError {
            code: "missing_query".to_string(),
            message: "missing 'query' parameter".to_string(),
        })?;
    let count = params
        .get("count")
        .and_then(Value::as_u64)
        .map(|n| n.clamp(1, 10))
        .and_then(|n| u8::try_from(n).ok())
        .unwrap_or(DEFAULT_RESULT_COUNT);
    let accept_language = params.get("_accept_language").and_then(Value::as_str);

    let mut url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={count}",
        url_encode(query)
    );
    if let Some(country) = params.get("country").and_then(Value::as_str) {
        url.push_str(&format!("&country={country}"));
    }
    if let Some(search_lang) = params.get("search_lang").and_then(Value::as_str) {
        url.push_str(&format!("&search_lang={search_lang}"));
    }
    if let Some(ui_lang) = params.get("ui_lang").and_then(Value::as_str) {
        url.push_str(&format!("&ui_lang={ui_lang}"));
    }
    if let Some(freshness) = params.get("freshness").and_then(Value::as_str) {
        url.push_str(&format!("&freshness={freshness}"));
    }

    let mut headers = vec![("Accept".to_string(), "application/json".to_string())];
    if let Some(lang) = accept_language {
        headers.push(("Accept-Language".to_string(), lang.to_string()));
    }
    let request = HttpRequest {
        method: "GET".to_string(),
        url,
        headers,
        body: None,
        timeout_ms: Some(DEFAULT_TIMEOUT_MS),
        max_response_bytes: Some(DEFAULT_MAX_RESPONSE_BYTES),
    };

    let response = outgoing_handler::handle(&request).map_err(map_http_error)?;
    if !(200..300).contains(&response.status) {
        return Ok(json!({
            "error": format!("HTTP {}", response.status),
            "query": query,
        }));
    }
    let body_text = String::from_utf8_lossy(&response.body).into_owned();
    let body_json: Value = serde_json::from_str(&body_text).map_err(|error| ToolError {
        code: "invalid_brave_json".to_string(),
        message: error.to_string(),
    })?;
    let results = parse_brave_results(&body_json);

    Ok(json!({
        "provider": "brave",
        "query": query,
        "results": results,
    }))
}

fn parse_brave_results(body: &Value) -> Vec<Value> {
    body.get("web")
        .and_then(|web| web.get("results"))
        .and_then(Value::as_array)
        .map(|results| {
            results
                .iter()
                .filter_map(|result| {
                    let title = result
                        .get("title")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .unwrap_or("");
                    let url = result
                        .get("url")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .unwrap_or("");
                    if title.is_empty() || url.is_empty() {
                        return None;
                    }
                    let description = result
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    Some(json!({
                        "title": title,
                        "url": url,
                        "description": description,
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(target_arch = "wasm32")]
fn map_http_error(error: HttpError) -> ToolError {
    match error {
        HttpError::InvalidUrl(message) => ToolError {
            code: "invalid_url".to_string(),
            message,
        },
        HttpError::BlockedUrl(message) => ToolError {
            code: "blocked_url".to_string(),
            message,
        },
        HttpError::Timeout(message) => ToolError {
            code: "timeout".to_string(),
            message,
        },
        HttpError::Network(message) => ToolError {
            code: "network".to_string(),
            message,
        },
        HttpError::Status(status) => ToolError {
            code: "http_status".to_string(),
            message: format!("HTTP {status}"),
        },
        HttpError::TooLarge(size) => ToolError {
            code: "too_large".to_string(),
            message: format!("response exceeded {size} bytes"),
        },
        HttpError::Other(message) => ToolError {
            code: "http_error".to_string(),
            message,
        },
    }
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            },
            _ => {
                out.push_str(&format!("%{byte:02X}"));
            },
        }
    }
    out
}

#[cfg(target_arch = "wasm32")]
export!(WebSearchWasm);

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    // --- url_encode ---

    #[test]
    fn encodes_spaces() {
        assert_eq!(url_encode("hello world"), "hello%20world");
    }

    #[test]
    fn preserves_unreserved_chars() {
        assert_eq!(url_encode("abc-123_XYZ.~"), "abc-123_XYZ.~");
    }

    #[test]
    fn encodes_special_chars() {
        assert_eq!(url_encode("a+b&c=d"), "a%2Bb%26c%3Dd");
    }

    #[test]
    fn encodes_unicode() {
        // "é" = [0xC3, 0xA9] in UTF-8
        assert_eq!(url_encode("é"), "%C3%A9");
    }

    #[test]
    fn empty_string() {
        assert_eq!(url_encode(""), "");
    }

    // --- parse_brave_results ---

    #[test]
    fn parses_normal_results() {
        let body = json!({
            "web": {
                "results": [
                    {
                        "title": "Example",
                        "url": "https://example.com",
                        "description": "An example site"
                    },
                    {
                        "title": "Rust Lang",
                        "url": "https://rust-lang.org",
                        "description": "The Rust programming language"
                    }
                ]
            }
        });
        let results = parse_brave_results(&body);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["title"], "Example");
        assert_eq!(results[0]["url"], "https://example.com");
        assert_eq!(results[1]["title"], "Rust Lang");
    }

    #[test]
    fn skips_results_missing_title() {
        let body = json!({
            "web": {
                "results": [
                    { "url": "https://example.com", "description": "no title" },
                    { "title": "Has title", "url": "https://example.com" }
                ]
            }
        });
        let results = parse_brave_results(&body);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["title"], "Has title");
    }

    #[test]
    fn skips_results_missing_url() {
        let body = json!({
            "web": {
                "results": [
                    { "title": "No URL" }
                ]
            }
        });
        let results = parse_brave_results(&body);
        assert!(results.is_empty());
    }

    #[test]
    fn skips_empty_title_and_url() {
        let body = json!({
            "web": {
                "results": [
                    { "title": "  ", "url": "https://example.com" },
                    { "title": "Good", "url": "  " }
                ]
            }
        });
        let results = parse_brave_results(&body);
        assert!(results.is_empty());
    }

    #[test]
    fn handles_missing_web_key() {
        let body = json!({ "other": "data" });
        let results = parse_brave_results(&body);
        assert!(results.is_empty());
    }

    #[test]
    fn handles_empty_results_array() {
        let body = json!({ "web": { "results": [] } });
        let results = parse_brave_results(&body);
        assert!(results.is_empty());
    }

    #[test]
    fn description_defaults_to_empty() {
        let body = json!({
            "web": {
                "results": [
                    { "title": "No desc", "url": "https://example.com" }
                ]
            }
        });
        let results = parse_brave_results(&body);
        assert_eq!(results[0]["description"], "");
    }
}
