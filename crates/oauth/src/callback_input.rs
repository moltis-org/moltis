use std::collections::HashMap;

use crate::{Error, Result};

/// Parsed OAuth callback payload extracted from user-provided input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCallbackInput {
    pub code: String,
    pub state: String,
}

/// Parse OAuth callback input copied from a browser or terminal.
///
/// Supported input formats:
/// - Full URL: `http://localhost:1455/auth/callback?code=...&state=...`
/// - Query string: `code=...&state=...`
/// - Compact token: `code#state`
pub fn parse_callback_input(raw: &str) -> Result<ParsedCallbackInput> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::message("callback input is empty"));
    }

    // Compact fallback format: `code#state`.
    if let Some((code, state)) = parse_compact_code_state(trimmed) {
        return Ok(ParsedCallbackInput { code, state });
    }

    // Full URL form.
    if let Ok(url) = url::Url::parse(trimmed) {
        if let Some(parsed) = parse_pairs(url.query()) {
            return Ok(parsed);
        }
        if let Some(parsed) = parse_pairs(url.fragment()) {
            return Ok(parsed);
        }
        return Err(Error::message(
            "callback URL must include both 'code' and 'state'",
        ));
    }

    // Raw query-string form.
    if let Some(parsed) = parse_pairs(Some(trimmed)) {
        return Ok(parsed);
    }

    Err(Error::message(
        "invalid callback input; expected URL, query string, or code#state",
    ))
}

fn parse_compact_code_state(value: &str) -> Option<(String, String)> {
    // Ignore URL fragments like `https://...#code=...`.
    if value.contains("://") {
        return None;
    }
    let (code, state) = value.split_once('#')?;
    if code.trim().is_empty() || state.trim().is_empty() {
        return None;
    }
    Some((code.trim().to_string(), state.trim().to_string()))
}

fn parse_pairs(raw: Option<&str>) -> Option<ParsedCallbackInput> {
    let raw = raw?;
    let raw = raw.trim().trim_start_matches('?').trim_start_matches('#');
    if raw.is_empty() {
        return None;
    }
    let pairs: HashMap<String, String> = url::form_urlencoded::parse(raw.as_bytes())
        .into_owned()
        .collect();
    let code = pairs.get("code")?.trim();
    let state = pairs.get("state")?.trim();
    if code.is_empty() || state.is_empty() {
        return None;
    }
    Some(ParsedCallbackInput {
        code: code.to_string(),
        state: state.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::parse_callback_input;

    #[test]
    fn parses_full_callback_url() {
        let parsed = match parse_callback_input(
            "http://localhost:1455/auth/callback?code=abc123&state=xyz789",
        ) {
            Ok(parsed) => parsed,
            Err(error) => panic!("should parse callback URL: {error}"),
        };
        assert_eq!(parsed.code, "abc123");
        assert_eq!(parsed.state, "xyz789");
    }

    #[test]
    fn parses_query_string() {
        let parsed = match parse_callback_input("code=abc123&state=xyz789") {
            Ok(parsed) => parsed,
            Err(error) => panic!("should parse query string: {error}"),
        };
        assert_eq!(parsed.code, "abc123");
        assert_eq!(parsed.state, "xyz789");
    }

    #[test]
    fn parses_compact_code_state() {
        let parsed = match parse_callback_input("abc123#xyz789") {
            Ok(parsed) => parsed,
            Err(error) => panic!("should parse compact form: {error}"),
        };
        assert_eq!(parsed.code, "abc123");
        assert_eq!(parsed.state, "xyz789");
    }

    #[test]
    fn rejects_missing_state() {
        let err = match parse_callback_input("http://localhost:1455/auth/callback?code=abc123") {
            Ok(parsed) => panic!("missing state should fail, got parsed input: {parsed:?}"),
            Err(error) => error,
        };
        assert!(
            err.to_string().contains("code") || err.to_string().contains("state"),
            "error should mention missing callback fields, got: {err}"
        );
    }
}
