// Pure functions are used by the WASM Guest impl; allow dead_code on non-wasm targets.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code, unused_imports))]

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

#[cfg(target_arch = "wasm32")]
use url::Url;

const DEFAULT_MAX_CHARS: usize = 50_000;
const DEFAULT_MAX_REDIRECTS: u8 = 5;
const DEFAULT_TIMEOUT_MS: u32 = 10_000;
const DEFAULT_MAX_RESPONSE_BYTES: u64 = 2_000_000;

#[cfg(target_arch = "wasm32")]
struct WebFetchWasm;

#[cfg(target_arch = "wasm32")]
impl Guest for WebFetchWasm {
    fn name() -> String {
        "web_fetch_wasm".to_string()
    }

    fn description() -> String {
        "Fetch a web URL through host HTTP capability and extract readable text.".to_string()
    }

    fn parameters_schema() -> String {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch (http/https)"
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

    let url_input = params
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError {
            code: "missing_url".to_string(),
            message: "missing 'url' parameter".to_string(),
        })?;

    let extract_mode = params
        .get("extract_mode")
        .and_then(Value::as_str)
        .unwrap_or("markdown");
    let max_chars = params
        .get("max_chars")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(DEFAULT_MAX_CHARS);
    let accept_language = params.get("_accept_language").and_then(Value::as_str);

    fetch_with_redirects(url_input, extract_mode, max_chars, accept_language)
}

#[cfg(target_arch = "wasm32")]
fn fetch_with_redirects(
    url_input: &str,
    extract_mode: &str,
    max_chars: usize,
    accept_language: Option<&str>,
) -> Result<Value, ToolError> {
    let mut current_url = Url::parse(url_input).map_err(|error| ToolError {
        code: "invalid_url".to_string(),
        message: error.to_string(),
    })?;
    if !matches!(current_url.scheme(), "http" | "https") {
        return Err(ToolError {
            code: "unsupported_scheme".to_string(),
            message: format!("unsupported URL scheme: {}", current_url.scheme()),
        });
    }

    let mut visited = vec![current_url.to_string()];
    let mut hops = 0_u8;

    loop {
        let mut headers = Vec::new();
        if let Some(lang) = accept_language {
            headers.push(("Accept-Language".to_string(), lang.to_string()));
        }
        let request = HttpRequest {
            method: "GET".to_string(),
            url: current_url.to_string(),
            headers,
            body: None,
            timeout_ms: Some(DEFAULT_TIMEOUT_MS),
            max_response_bytes: Some(DEFAULT_MAX_RESPONSE_BYTES),
        };

        let response = outgoing_handler::handle(&request).map_err(map_http_error)?;
        if (300..400).contains(&response.status) {
            if hops >= DEFAULT_MAX_REDIRECTS {
                return Err(ToolError {
                    code: "too_many_redirects".to_string(),
                    message: format!("too many redirects (max {DEFAULT_MAX_REDIRECTS})"),
                });
            }
            let location =
                header_value(&response.headers, "location").ok_or_else(|| ToolError {
                    code: "redirect_missing_location".to_string(),
                    message: "redirect without Location header".to_string(),
                })?;
            let next = current_url.join(&location).map_err(|error| ToolError {
                code: "redirect_invalid_location".to_string(),
                message: error.to_string(),
            })?;
            let next_s = next.to_string();
            if visited.iter().any(|seen| seen == &next_s) {
                return Err(ToolError {
                    code: "redirect_loop".to_string(),
                    message: format!("redirect loop detected at {next_s}"),
                });
            }
            visited.push(next_s);
            current_url = next;
            hops = hops.saturating_add(1);
            continue;
        }

        if !(200..300).contains(&response.status) {
            return Ok(json!({
                "error": format!("HTTP {}", response.status),
                "url": current_url.to_string(),
            }));
        }

        let content_type = response.content_type.unwrap_or_default();
        let body = String::from_utf8_lossy(&response.body).into_owned();
        let (content, detected_mode) = extract_content(&body, &content_type, extract_mode);
        let truncated = content.len() > max_chars;
        let content = if truncated {
            truncate_at_char_boundary(&content, max_chars)
        } else {
            content
        };
        return Ok(json!({
            "url": current_url.to_string(),
            "content_type": content_type,
            "extract_mode": detected_mode,
            "content": content,
            "truncated": truncated,
            "original_length": body.len(),
        }));
    }
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

fn header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
}

fn extract_content(body: &str, content_type: &str, requested_mode: &str) -> (String, String) {
    let ct_lower = content_type.to_ascii_lowercase();
    if ct_lower.contains("json") {
        if let Ok(parsed) = serde_json::from_str::<Value>(body) {
            let pretty = serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| body.to_string());
            return (pretty, "json".to_string());
        }
        return (body.to_string(), "text".to_string());
    }
    if ct_lower.contains("text/plain") || !ct_lower.contains("html") {
        return (body.to_string(), "text".to_string());
    }
    if requested_mode == "markdown" || requested_mode.is_empty() {
        return (html_to_text(body), "markdown".to_string());
    }
    (html_to_text(body), "text".to_string())
}

fn html_to_text(html: &str) -> String {
    let mut result = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let mut last_was_space = false;
    let html_lower = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    let lower_bytes = html_lower.as_bytes();

    let mut index = 0_usize;
    while index < bytes.len() {
        if bytes[index] == b'<' {
            if index + 7 < lower_bytes.len() && &lower_bytes[index..index + 7] == b"<script" {
                in_script = true;
            }
            if index + 9 < lower_bytes.len() && &lower_bytes[index..index + 9] == b"</script>" {
                in_script = false;
            }
            if index + 6 < lower_bytes.len() && &lower_bytes[index..index + 6] == b"<style" {
                in_style = true;
            }
            if index + 8 < lower_bytes.len() && &lower_bytes[index..index + 8] == b"</style>" {
                in_style = false;
            }
            in_tag = true;
            index += 1;
            continue;
        }

        if bytes[index] == b'>' {
            in_tag = false;
            index += 1;
            continue;
        }

        if in_tag || in_script || in_style {
            index += 1;
            continue;
        }

        let ch = bytes[index] as char;
        if ch.is_ascii_whitespace() {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(ch);
            last_was_space = false;
        }
        index += 1;
    }

    result.trim().to_string()
}

fn truncate_at_char_boundary(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

#[cfg(target_arch = "wasm32")]
export!(WebFetchWasm);

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    // --- html_to_text ---

    #[test]
    fn strips_tags() {
        assert_eq!(html_to_text("<p>hello</p>"), "hello");
    }

    #[test]
    fn strips_script_content() {
        let html = "<p>before</p><script>var x = 1;</script><p>after</p>";
        let text = html_to_text(html);
        assert!(!text.contains("var x"));
        assert!(text.contains("before"));
        assert!(text.contains("after"));
    }

    #[test]
    fn strips_style_content() {
        let html = "<style>.foo { color: red; }</style><p>visible</p>";
        let text = html_to_text(html);
        assert!(!text.contains("color"));
        assert!(text.contains("visible"));
    }

    #[test]
    fn collapses_whitespace() {
        assert_eq!(html_to_text("hello   \n\t  world"), "hello world");
    }

    #[test]
    fn empty_html() {
        assert_eq!(html_to_text(""), "");
    }

    #[test]
    fn nested_tags() {
        assert_eq!(
            html_to_text("<div><span>nested</span> text</div>"),
            "nested text"
        );
    }

    // --- extract_content ---

    #[test]
    fn json_content_type_prettifies() {
        let (content, mode) = extract_content(r#"{"a":1}"#, "application/json", "markdown");
        assert_eq!(mode, "json");
        assert!(content.contains("\"a\": 1"));
    }

    #[test]
    fn invalid_json_falls_back_to_text() {
        let (content, mode) = extract_content("not json", "application/json", "markdown");
        assert_eq!(mode, "text");
        assert_eq!(content, "not json");
    }

    #[test]
    fn plain_text_passthrough() {
        let (content, mode) = extract_content("hello world", "text/plain", "markdown");
        assert_eq!(mode, "text");
        assert_eq!(content, "hello world");
    }

    #[test]
    fn html_markdown_mode() {
        let (content, mode) = extract_content("<p>hello</p>", "text/html", "markdown");
        assert_eq!(mode, "markdown");
        assert_eq!(content, "hello");
    }

    #[test]
    fn html_text_mode() {
        let (content, mode) = extract_content("<p>hello</p>", "text/html", "text");
        assert_eq!(mode, "text");
        assert_eq!(content, "hello");
    }

    #[test]
    fn html_empty_mode_defaults_to_markdown() {
        let (_content, mode) = extract_content("<p>hi</p>", "text/html", "");
        assert_eq!(mode, "markdown");
    }

    #[test]
    fn unknown_content_type_treated_as_text() {
        let (content, mode) = extract_content("raw bytes", "application/octet-stream", "markdown");
        assert_eq!(mode, "text");
        assert_eq!(content, "raw bytes");
    }

    // --- truncate_at_char_boundary ---

    #[test]
    fn no_truncation_needed() {
        assert_eq!(truncate_at_char_boundary("hello", 10), "hello");
    }

    #[test]
    fn truncates_ascii() {
        assert_eq!(truncate_at_char_boundary("hello world", 5), "hello");
    }

    #[test]
    fn truncates_at_valid_utf8_boundary() {
        // U+00E9 (é) is 2 bytes in UTF-8: "café!" = [99, 97, 102, 195, 169, 33]
        // Truncating at byte 4 lands inside é — backs up to byte 3
        let s = "caf\u{00e9}!";
        assert_eq!(truncate_at_char_boundary(s, 4), "caf");
        // Truncating at byte 5 lands after é — includes the full char
        assert_eq!(truncate_at_char_boundary(s, 5), "caf\u{00e9}");
    }

    #[test]
    fn truncate_multibyte_backs_up() {
        // "café" = 5 bytes, max=4 lands inside é → backs up to "caf"
        let s = "caf\u{00e9}";
        assert_eq!(truncate_at_char_boundary(s, 4), "caf");
    }

    // --- header_value ---

    #[test]
    fn finds_header_case_insensitive() {
        let headers = vec![
            ("Content-Type".to_string(), "text/html".to_string()),
            ("X-Custom".to_string(), "value".to_string()),
        ];
        assert_eq!(
            header_value(&headers, "content-type"),
            Some("text/html".to_string())
        );
    }

    #[test]
    fn missing_header_returns_none() {
        let headers: Vec<(String, String)> = vec![];
        assert_eq!(header_value(&headers, "content-type"), None);
    }
}
