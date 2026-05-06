//! Parsing helpers for MCP server configuration from JSON parameters.
//!
//! Extracted from the gateway's `mcp_service` module. These are pure
//! functions with no gateway state dependency.

use std::collections::HashMap;

use {
    secrecy::{ExposeSecret, Secret},
    serde_json::Value,
};

use crate::{
    McpServerConfig, TransportType,
    error::{Error, Result},
    registry::McpOAuthConfig,
};

/// Extract an [`McpServerConfig`] from JSON params.
///
/// For updates, omitted fields inherit from `existing`.
pub fn parse_server_config(
    params: &Value,
    existing: Option<&McpServerConfig>,
) -> Result<McpServerConfig> {
    let transport = match params.get("transport").and_then(|v| v.as_str()) {
        Some("sse") => TransportType::Sse,
        Some("streamable-http" | "streamable_http" | "http") => TransportType::StreamableHttp,
        Some(_) => TransportType::Stdio,
        None => existing
            .map(|cfg| cfg.transport)
            .unwrap_or(TransportType::Stdio),
    };

    let command = params
        .get("command")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| existing.map(|cfg| cfg.command.clone()))
        .unwrap_or_default();

    if matches!(transport, TransportType::Stdio) && command.trim().is_empty() {
        return Err(Error::message("missing 'command' parameter"));
    }

    let args: Vec<String> = if params.get("args").is_some() {
        params
            .get("args")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    } else {
        existing.map(|cfg| cfg.args.clone()).unwrap_or_default()
    };

    let enabled = params
        .get("enabled")
        .and_then(|v| v.as_bool())
        .or_else(|| existing.map(|cfg| cfg.enabled))
        .unwrap_or(true);

    let request_timeout_secs = if let Some(v) = params.get("request_timeout_secs") {
        if v.is_null() {
            None
        } else {
            let secs = v
                .as_u64()
                .ok_or_else(|| Error::message("invalid 'request_timeout_secs' parameter"))?;
            if secs == 0 {
                return Err(Error::message(
                    "'request_timeout_secs' must be greater than 0",
                ));
            }
            Some(secs)
        }
    } else {
        existing.and_then(|cfg| cfg.request_timeout_secs)
    };

    let url = if params.get("url").is_some() {
        if params.get("url").is_some_and(Value::is_null) {
            None
        } else {
            params
                .get("url")
                .and_then(|v| v.as_str())
                .map(|value| Secret::new(value.to_string()))
        }
    } else {
        existing.and_then(|cfg| cfg.url.clone())
    };

    let headers = if matches!(
        transport,
        TransportType::Sse | TransportType::StreamableHttp
    ) {
        if params.get("headers").is_some() {
            parse_secret_string_map(params.get("headers").unwrap_or(&Value::Null))
        } else {
            existing.map(|cfg| cfg.headers.clone()).unwrap_or_default()
        }
    } else {
        HashMap::new()
    };

    let env = if matches!(
        transport,
        TransportType::Sse | TransportType::StreamableHttp
    ) {
        HashMap::new()
    } else if params.get("env").is_some() {
        parse_string_map(params.get("env").unwrap_or(&Value::Null))
    } else {
        existing.map(|cfg| cfg.env.clone()).unwrap_or_default()
    };

    if matches!(
        transport,
        TransportType::Sse | TransportType::StreamableHttp
    ) && url
        .as_ref()
        .map(ExposeSecret::expose_secret)
        .is_none_or(|candidate| candidate.trim().is_empty())
    {
        return Err(Error::message(format!(
            "missing 'url' parameter for '{}' transport",
            transport
        )));
    }

    let oauth = if let Some(v) = params.get("oauth") {
        if v.is_null() {
            None
        } else {
            let client_id = v
                .get("client_id")
                .and_then(|val| val.as_str())
                .ok_or_else(|| Error::message("missing 'oauth.client_id' parameter"))?
                .to_string();
            let auth_url = v
                .get("auth_url")
                .and_then(|val| val.as_str())
                .ok_or_else(|| Error::message("missing 'oauth.auth_url' parameter"))?
                .to_string();
            let token_url = v
                .get("token_url")
                .and_then(|val| val.as_str())
                .ok_or_else(|| Error::message("missing 'oauth.token_url' parameter"))?
                .to_string();
            let scopes: Vec<String> = v
                .get("scopes")
                .and_then(|s| serde_json::from_value(s.clone()).ok())
                .unwrap_or_default();
            Some(McpOAuthConfig {
                client_id,
                auth_url,
                token_url,
                scopes,
            })
        }
    } else {
        existing.and_then(|cfg| cfg.oauth.clone())
    };

    let display_name = match params.get("display_name") {
        Some(v) if v.is_null() => None,
        Some(v) => v.as_str().map(String::from),
        None => existing.and_then(|cfg| cfg.display_name.clone()),
    };

    Ok(McpServerConfig {
        command,
        args,
        env,
        enabled,
        request_timeout_secs,
        transport,
        url: if matches!(
            transport,
            TransportType::Sse | TransportType::StreamableHttp
        ) {
            url
        } else {
            None
        },
        headers,
        oauth,
        display_name,
    })
}

fn parse_string_map(value: &Value) -> HashMap<String, String> {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

fn parse_secret_string_map(value: &Value) -> HashMap<String, Secret<String>> {
    parse_string_map(value)
        .into_iter()
        .map(|(key, value)| (key, Secret::new(value)))
        .collect()
}

/// Merge environment overrides, keeping base config values authoritative.
///
/// Config `[env]` values stay authoritative so checked-in config cannot be
/// silently shadowed by mutable UI-managed entries from the credential store.
pub fn merge_env_overrides(
    base_overrides: &HashMap<String, String>,
    additional: Vec<(String, String)>,
) -> HashMap<String, String> {
    let mut merged = base_overrides.clone();
    for (key, value) in additional {
        if key.trim().is_empty() || value.trim().is_empty() {
            continue;
        }
        merged.entry(key).or_insert(value);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_server_config ────────────────────────────────────────────────

    #[test]
    fn parse_server_config_allows_sse_without_command() {
        let cfg = parse_server_config(
            &serde_json::json!({
                "transport": "sse",
                "url": "http://localhost:8080/mcp"
            }),
            None,
        )
        .unwrap();
        assert_eq!(cfg.transport, TransportType::Sse);
        assert_eq!(
            cfg.url.as_ref().map(|u| u.expose_secret().as_str()),
            Some("http://localhost:8080/mcp")
        );
    }

    #[test]
    fn parse_server_config_requires_command_for_stdio() {
        let err = parse_server_config(
            &serde_json::json!({
                "transport": "stdio"
            }),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("missing 'command'"));
    }

    #[test]
    fn parse_server_config_requires_url_for_sse() {
        let err = parse_server_config(
            &serde_json::json!({
                "transport": "sse"
            }),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("missing 'url'"));
    }

    #[test]
    fn parse_server_config_allows_streamable_http_without_command() {
        let cfg = parse_server_config(
            &serde_json::json!({
                "transport": "streamable-http",
                "url": "http://localhost:8080/mcp"
            }),
            None,
        )
        .unwrap();
        assert_eq!(cfg.transport, TransportType::StreamableHttp);
        assert_eq!(
            cfg.url.as_ref().map(|u| u.expose_secret().as_str()),
            Some("http://localhost:8080/mcp")
        );
    }

    #[test]
    fn parse_server_config_requires_url_for_streamable_http() {
        let err = parse_server_config(
            &serde_json::json!({
                "transport": "streamable-http"
            }),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("missing 'url'"));
    }

    #[test]
    fn parse_server_config_update_preserves_existing_sse_fields() {
        let existing = McpServerConfig {
            command: String::new(),
            args: vec![],
            env: HashMap::new(),
            enabled: true,
            request_timeout_secs: None,
            transport: TransportType::Sse,
            url: Some(Secret::new("http://example.com".to_string())),
            headers: {
                let mut h = HashMap::new();
                h.insert(
                    "Authorization".to_string(),
                    Secret::new("Bearer old".to_string()),
                );
                h
            },
            oauth: None,
            display_name: Some("My Server".to_string()),
        };

        // Update only display_name, rest should be preserved
        let cfg = parse_server_config(
            &serde_json::json!({
                "display_name": "Updated Server"
            }),
            Some(&existing),
        )
        .unwrap();

        assert_eq!(cfg.display_name, Some("Updated Server".to_string()));
        assert_eq!(
            cfg.url.as_ref().map(|u| u.expose_secret().as_str()),
            Some("http://example.com")
        );
        assert_eq!(
            cfg.headers
                .get("Authorization")
                .map(|h| h.expose_secret().as_str()),
            Some("Bearer old")
        );
    }

    #[test]
    fn parse_server_config_update_preserves_oauth_when_omitted() {
        let existing = McpServerConfig {
            command: String::new(),
            args: vec![],
            env: HashMap::new(),
            enabled: true,
            request_timeout_secs: None,
            transport: TransportType::Sse,
            url: Some(Secret::new("http://example.com".to_string())),
            headers: HashMap::new(),
            oauth: Some(McpOAuthConfig {
                client_id: "test-client".to_string(),
                auth_url: "http://auth.example.com".to_string(),
                token_url: "http://token.example.com".to_string(),
                scopes: vec!["read".to_string()],
            }),
            display_name: None,
        };

        let cfg = parse_server_config(
            &serde_json::json!({
                "display_name": "Updated"
            }),
            Some(&existing),
        )
        .unwrap();

        let oauth = cfg.oauth.as_ref().unwrap();
        assert_eq!(oauth.client_id, "test-client");
        assert_eq!(oauth.scopes, vec!["read"]);
    }

    #[test]
    fn parse_server_config_preserves_and_replaces_sse_headers() {
        let existing = McpServerConfig {
            command: String::new(),
            args: vec![],
            env: HashMap::new(),
            enabled: true,
            request_timeout_secs: None,
            transport: TransportType::Sse,
            url: Some(Secret::new("http://example.com".to_string())),
            headers: {
                let mut h = HashMap::new();
                h.insert("X-Keep".to_string(), Secret::new("kept".to_string()));
                h.insert("X-Replace".to_string(), Secret::new("old".to_string()));
                h
            },
            oauth: None,
            display_name: None,
        };

        // Replace X-Replace, don't mention X-Keep
        let cfg = parse_server_config(
            &serde_json::json!({
                "headers": {
                    "X-Replace": "new"
                }
            }),
            Some(&existing),
        )
        .unwrap();

        assert_eq!(
            cfg.headers
                .get("X-Replace")
                .map(|h| h.expose_secret().as_str()),
            Some("new")
        );
        // X-Keep should NOT be preserved — headers are fully replaced
        assert!(!cfg.headers.contains_key("X-Keep"));
    }

    #[test]
    fn parse_server_config_allows_clearing_sse_headers() {
        let existing = McpServerConfig {
            command: String::new(),
            args: vec![],
            env: HashMap::new(),
            enabled: true,
            request_timeout_secs: None,
            transport: TransportType::Sse,
            url: Some(Secret::new("http://example.com".to_string())),
            headers: {
                let mut h = HashMap::new();
                h.insert(
                    "Authorization".to_string(),
                    Secret::new("Bearer old".to_string()),
                );
                h
            },
            oauth: None,
            display_name: None,
        };

        let cfg = parse_server_config(
            &serde_json::json!({
                "headers": {}
            }),
            Some(&existing),
        )
        .unwrap();

        assert!(cfg.headers.is_empty());
    }

    #[test]
    fn merge_env_overrides_keeps_config_values_authoritative() {
        let base: HashMap<String, String> = vec![
            ("API_KEY".to_string(), "from-config".to_string()),
            ("MODEL".to_string(), "from-config".to_string()),
        ]
        .into_iter()
        .collect();

        let merged = merge_env_overrides(&base, vec![
            ("API_KEY".to_string(), "from-db".to_string()),
            ("NEW_VAR".to_string(), "from-db".to_string()),
            ("".to_string(), "ignored-empty-key".to_string()),
            ("EMPTY_VAL".to_string(), "".to_string()),
        ]);

        assert_eq!(merged.get("API_KEY").unwrap(), "from-config"); // base wins
        assert_eq!(merged.get("NEW_VAR").unwrap(), "from-db"); // new from additional
        assert_eq!(merged.get("MODEL").unwrap(), "from-config"); // unchanged
        assert!(!merged.contains_key("")); // empty key skipped
        assert!(!merged.contains_key("EMPTY_VAL")); // empty val skipped
    }

    #[test]
    fn parse_server_config_preserves_request_timeout_override() {
        let existing = McpServerConfig {
            command: "uvx".to_string(),
            args: vec!["mcp-server".to_string()],
            env: HashMap::new(),
            enabled: true,
            request_timeout_secs: Some(60),
            transport: TransportType::Stdio,
            url: None,
            headers: HashMap::new(),
            oauth: None,
            display_name: None,
        };

        // Omit timeout — should preserve existing
        let cfg = parse_server_config(
            &serde_json::json!({
                "display_name": "test"
            }),
            Some(&existing),
        )
        .unwrap();
        assert_eq!(cfg.request_timeout_secs, Some(60));

        // Explicit null — should clear
        let cfg = parse_server_config(
            &serde_json::json!({
                "request_timeout_secs": null
            }),
            Some(&existing),
        )
        .unwrap();
        assert_eq!(cfg.request_timeout_secs, None);

        // New value
        let cfg = parse_server_config(
            &serde_json::json!({
                "request_timeout_secs": 120
            }),
            Some(&existing),
        )
        .unwrap();
        assert_eq!(cfg.request_timeout_secs, Some(120));
    }
}
