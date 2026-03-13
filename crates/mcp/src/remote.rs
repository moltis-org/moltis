use std::{collections::HashMap, str::FromStr};

use {
    reqwest::header::{HeaderMap, HeaderName, HeaderValue},
    secrecy::{ExposeSecret, Secret},
    url::Url,
};

use crate::{
    error::{Context, Error, Result},
    registry::McpServerConfig,
};

const REDACTED_QUERY_VALUE: &str = "[REDACTED]";
const REDACTED_USERINFO: &str = "redacted";
const INVALID_REMOTE_URL: &str = "[invalid remote URL]";

#[derive(Clone)]
pub struct ResolvedRemoteConfig {
    request_url: Secret<String>,
    display_url: String,
    headers: HeaderMap,
}

impl ResolvedRemoteConfig {
    pub fn from_server_config(
        config: &McpServerConfig,
        env_overrides: &HashMap<String, String>,
    ) -> Result<Self> {
        let raw_url = config.url.as_ref().context("missing remote MCP url")?;
        let display_url = sanitize_url_for_display(raw_url.expose_secret());
        let request_url = substitute_env_placeholders(raw_url.expose_secret(), env_overrides)
            .trim()
            .to_string();

        Url::parse(&request_url)
            .map_err(|_| Error::message(format!("invalid remote MCP url '{}'", display_url)))?;

        let headers = build_header_map(&config.headers, env_overrides)?;

        Ok(Self {
            request_url: Secret::new(request_url),
            display_url,
            headers,
        })
    }

    pub fn request_url(&self) -> &str {
        self.request_url.expose_secret()
    }

    pub fn display_url(&self) -> &str {
        &self.display_url
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }
}

pub fn header_names(headers: &HashMap<String, Secret<String>>) -> Vec<String> {
    let mut names: Vec<String> = headers.keys().cloned().collect();
    names.sort();
    names
}

pub fn sanitize_url_for_display(raw_url: &str) -> String {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let Ok(mut url) = Url::parse(trimmed) else {
        return INVALID_REMOTE_URL.to_string();
    };

    if !url.username().is_empty() || url.password().is_some() {
        let _ = url.set_username(REDACTED_USERINFO);
        let _ = url.set_password(None);
    }

    if url.query().is_some() {
        let redacted_pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(key, value)| {
                let value = if value.is_empty() || is_entire_env_placeholder_syntax(&value) {
                    value.into_owned()
                } else {
                    REDACTED_QUERY_VALUE.to_string()
                };
                (key.into_owned(), value)
            })
            .collect();

        {
            let mut query = url.query_pairs_mut();
            query.clear();
            query.extend_pairs(
                redacted_pairs
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.as_str())),
            );
        }
    }

    decode_display_escapes(url.as_ref())
}

pub fn sanitize_reqwest_error(err: reqwest::Error) -> reqwest::Error {
    err.without_url()
}

pub fn substitute_env_placeholders(input: &str, env_overrides: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut cursor = 0usize;

    while let Some(relative_dollar) = input[cursor..].find('$') {
        let dollar_idx = cursor + relative_dollar;
        out.push_str(&input[cursor..dollar_idx]);

        let after_dollar = dollar_idx + 1;
        if after_dollar >= input.len() {
            out.push('$');
            cursor = after_dollar;
            break;
        }

        if bytes[after_dollar] == b'{' {
            let name_start = after_dollar + 1;
            let Some(relative_close) = input[name_start..].find('}') else {
                out.push_str(&input[dollar_idx..]);
                cursor = input.len();
                break;
            };
            let close_idx = name_start + relative_close;
            if close_idx > name_start {
                let name = &input[name_start..close_idx];
                if let Some(value) = lookup_env(name, env_overrides) {
                    out.push_str(&value);
                } else {
                    out.push_str(&input[dollar_idx..=close_idx]);
                }
                cursor = close_idx + 1;
                continue;
            }

            out.push_str(&input[dollar_idx..]);
            break;
        }

        let next = bytes[after_dollar];
        if !is_env_ident_start(next as char) {
            out.push('$');
            cursor = after_dollar;
            continue;
        }

        let name_start = after_dollar;
        let mut name_end = name_start + 1;
        while name_end < input.len() && is_env_ident_continue(bytes[name_end] as char) {
            name_end += 1;
        }

        let name = &input[name_start..name_end];
        if let Some(value) = lookup_env(name, env_overrides) {
            out.push_str(&value);
        } else {
            out.push_str(&input[dollar_idx..name_end]);
        }
        cursor = name_end;
    }

    out.push_str(&input[cursor..]);
    out
}

fn build_header_map(
    headers: &HashMap<String, Secret<String>>,
    env_overrides: &HashMap<String, String>,
) -> Result<HeaderMap> {
    let mut resolved = HeaderMap::new();
    let mut ordered_headers: Vec<_> = headers.iter().collect();
    ordered_headers.sort_by_cached_key(|(name, _)| (name.to_ascii_lowercase(), (*name).clone()));

    for (name, raw_value) in ordered_headers {
        if is_reserved_header(name) {
            return Err(Error::message(format!(
                "remote MCP header '{}' is reserved",
                name
            )));
        }

        let header_name = HeaderName::from_str(name).map_err(|error| {
            Error::message(format!(
                "invalid remote MCP header name '{}': {error}",
                name
            ))
        })?;
        if resolved.contains_key(&header_name) {
            return Err(Error::message(format!(
                "duplicate remote MCP header '{}' conflicts case-insensitively with another header",
                name
            )));
        }
        let value = substitute_env_placeholders(raw_value.expose_secret(), env_overrides);
        let header_value = HeaderValue::from_str(&value).map_err(|error| {
            Error::message(format!(
                "invalid remote MCP header value for '{}': {error}",
                name
            ))
        })?;
        resolved.insert(header_name, header_value);
    }

    Ok(resolved)
}

fn lookup_env(name: &str, env_overrides: &HashMap<String, String>) -> Option<String> {
    env_overrides
        .get(name)
        .cloned()
        .filter(|value| !value.trim().is_empty())
}

fn is_entire_env_placeholder_syntax(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }

    if let Some(name) = trimmed
        .strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))
    {
        return !name.is_empty() && name.chars().all(is_env_ident_continue);
    }

    trimmed
        .strip_prefix('$')
        .is_some_and(|name| !name.is_empty() && name.chars().all(is_env_ident_continue))
}

fn decode_display_escapes(value: &str) -> String {
    [
        ("%5B", "["),
        ("%5D", "]"),
        ("%24", "$"),
        ("%7B", "{"),
        ("%7D", "}"),
        ("%5b", "["),
        ("%5d", "]"),
        ("%7b", "{"),
        ("%7d", "}"),
    ]
    .into_iter()
    .fold(value.to_string(), |acc, (encoded, decoded)| {
        acc.replace(encoded, decoded)
    })
}

fn is_env_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_env_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn is_reserved_header(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "accept" | "content-type" | "mcp-protocol-version" | "mcp-session-id"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitute_env_placeholders_supports_braced_and_bare_keys() {
        let overrides = HashMap::from([
            ("ONE".to_string(), "alpha".to_string()),
            ("TWO".to_string(), "beta".to_string()),
        ]);

        let value = substitute_env_placeholders("x=${ONE}&y=$TWO", &overrides);
        assert_eq!(value, "x=alpha&y=beta");
    }

    #[test]
    fn sanitize_url_for_display_redacts_query_values() {
        let sanitized =
            sanitize_url_for_display("https://example.com/mcp?api_key=secret&mode=fast");
        assert!(sanitized.contains("api_key=[REDACTED]"));
        assert!(sanitized.contains("mode=[REDACTED]"));
        assert!(!sanitized.contains("secret"));
        assert!(!sanitized.contains("fast"));
    }

    #[test]
    fn sanitize_url_for_display_preserves_placeholders() {
        let sanitized =
            sanitize_url_for_display("https://example.com/mcp?api_key=${THEKEY}&mode=$MODE");
        assert!(sanitized.contains("api_key=${THEKEY}"));
        assert!(sanitized.contains("mode=$MODE"));
    }

    #[test]
    fn sanitize_url_for_display_redacts_mixed_placeholder_values() {
        let sanitized =
            sanitize_url_for_display("https://example.com/mcp?api_key=prefix-${THEKEY}");
        assert!(sanitized.contains("api_key=[REDACTED]"));
        assert!(!sanitized.contains("prefix-"));
    }

    #[test]
    fn substitute_env_placeholders_ignores_process_env() {
        let value = substitute_env_placeholders("token=$HOME", &HashMap::new());
        assert_eq!(value, "token=$HOME");
    }

    #[test]
    fn substitute_env_placeholders_preserves_utf8_segments() {
        let overrides = HashMap::from([("TOKEN".to_string(), "alpha".to_string())]);

        let value = substitute_env_placeholders(
            "https://example.com/cafe\u{301}?token=$TOKEN&label=na\u{ef}ve",
            &overrides,
        );

        assert_eq!(
            value,
            "https://example.com/cafe\u{301}?token=alpha&label=na\u{ef}ve"
        );
    }

    #[test]
    fn build_header_map_rejects_case_insensitive_duplicates() {
        let headers = HashMap::from([
            ("X-Workspace".to_string(), Secret::new("alpha".to_string())),
            ("x-workspace".to_string(), Secret::new("beta".to_string())),
        ]);

        let error = build_header_map(&headers, &HashMap::new())
            .err()
            .map(|err| err.to_string());

        assert_eq!(
            error.as_deref(),
            Some(
                "duplicate remote MCP header 'x-workspace' conflicts case-insensitively with another header"
            )
        );
    }
}
