//! Configuration validation engine.
//!
//! Validates TOML configuration files against the known schema, detects
//! unknown/misspelled fields, and reports security warnings.

use std::{collections::HashMap, path::Path};

use crate::schema::MoltisConfig;

/// Severity level for a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error => write!(f, "error"),
            Self::Warning => write!(f, "warning"),
            Self::Info => write!(f, "info"),
        }
    }
}

/// A single validation diagnostic.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Category: "syntax", "unknown-field", "unknown-provider", "type-error",
    /// "security", "file-ref"
    pub category: &'static str,
    /// Dotted path, e.g. "server.bnd"
    pub path: String,
    pub message: String,
}

/// Result of validating a configuration file.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub diagnostics: Vec<Diagnostic>,
    pub config_path: Option<std::path::PathBuf>,
}

impl ValidationResult {
    /// Returns `true` if any diagnostic is an error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }

    /// Count diagnostics by severity.
    #[must_use]
    pub fn count(&self, severity: Severity) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == severity)
            .count()
    }
}

// ── Schema tree for unknown-field detection ─────────────────────────────────

/// Represents the expected shape of the configuration schema.
enum KnownKeys {
    /// A struct with fixed field names.
    Struct(HashMap<&'static str, KnownKeys>),
    /// A map with dynamic keys (providers, mcp.servers, etc.) whose values
    /// have a known shape.
    Map(Box<KnownKeys>),
    /// A map with dynamic keys plus explicit static keys.
    MapWithFields {
        value: Box<KnownKeys>,
        fields: HashMap<&'static str, KnownKeys>,
    },
    /// An array of typed items.
    Array(Box<KnownKeys>),
    /// Scalar value — stop recursion.
    Leaf,
}

/// Known LLM provider names for hint diagnostics.
const KNOWN_PROVIDER_NAMES: &[&str] = &[
    "anthropic",
    "openai",
    "gemini",
    "groq",
    "xai",
    "deepseek",
    "mistral",
    "openrouter",
    "cerebras",
    "minimax",
    "moonshot",
    "venice",
    "ollama",
];

/// Static metadata keys allowed directly under `[providers]`.
const PROVIDERS_META_KEYS: &[&str] = &["offered"];

/// Build the full schema map mirroring every field in `schema.rs`.
fn build_schema_map() -> KnownKeys {
    use KnownKeys::{Array, Leaf, Map, MapWithFields, Struct};

    let provider_entry = || {
        Struct(HashMap::from([
            ("enabled", Leaf),
            ("api_key", Leaf),
            ("base_url", Leaf),
            ("model", Leaf),
            ("alias", Leaf),
        ]))
    };

    let resource_limits = || {
        Struct(HashMap::from([
            ("memory_limit", Leaf),
            ("cpu_quota", Leaf),
            ("pids_max", Leaf),
        ]))
    };

    let sandbox = || {
        Struct(HashMap::from([
            ("mode", Leaf),
            ("scope", Leaf),
            ("workspace_mount", Leaf),
            ("image", Leaf),
            ("container_prefix", Leaf),
            ("no_network", Leaf),
            ("backend", Leaf),
            ("resource_limits", resource_limits()),
            ("packages", Leaf),
        ]))
    };

    let perplexity = || {
        Struct(HashMap::from([
            ("api_key", Leaf),
            ("base_url", Leaf),
            ("model", Leaf),
        ]))
    };

    let web_search = || {
        Struct(HashMap::from([
            ("enabled", Leaf),
            ("provider", Leaf),
            ("api_key", Leaf),
            ("max_results", Leaf),
            ("timeout_seconds", Leaf),
            ("cache_ttl_minutes", Leaf),
            ("perplexity", perplexity()),
        ]))
    };

    let web_fetch = || {
        Struct(HashMap::from([
            ("enabled", Leaf),
            ("max_chars", Leaf),
            ("timeout_seconds", Leaf),
            ("cache_ttl_minutes", Leaf),
            ("max_redirects", Leaf),
            ("readability", Leaf),
        ]))
    };

    let exec = || {
        Struct(HashMap::from([
            ("default_timeout_secs", Leaf),
            ("max_output_bytes", Leaf),
            ("approval_mode", Leaf),
            ("security_level", Leaf),
            ("allowlist", Leaf),
            ("sandbox", sandbox()),
        ]))
    };

    let browser = || {
        Struct(HashMap::from([
            ("enabled", Leaf),
            ("chrome_path", Leaf),
            ("headless", Leaf),
            ("viewport_width", Leaf),
            ("viewport_height", Leaf),
            ("device_scale_factor", Leaf),
            ("max_instances", Leaf),
            ("memory_limit_percent", Leaf),
            ("idle_timeout_secs", Leaf),
            ("navigation_timeout_ms", Leaf),
            ("user_agent", Leaf),
            ("chrome_args", Leaf),
            ("sandbox", Leaf),
            ("sandbox_image", Leaf),
            ("allowed_domains", Leaf),
        ]))
    };

    let tools = || {
        Struct(HashMap::from([
            ("exec", exec()),
            ("browser", browser()),
            (
                "policy",
                Struct(HashMap::from([
                    ("allow", Leaf),
                    ("deny", Leaf),
                    ("profile", Leaf),
                ])),
            ),
            (
                "web",
                Struct(HashMap::from([
                    ("search", web_search()),
                    ("fetch", web_fetch()),
                ])),
            ),
            ("agent_timeout_secs", Leaf),
            ("max_tool_result_bytes", Leaf),
        ]))
    };

    let mcp_server_entry = || {
        Struct(HashMap::from([
            ("command", Leaf),
            ("args", Leaf),
            ("env", Map(Box::new(Leaf))),
            ("enabled", Leaf),
            ("transport", Leaf),
            ("url", Leaf),
        ]))
    };

    let shell_hook_entry = || {
        Struct(HashMap::from([
            ("name", Leaf),
            ("command", Leaf),
            ("events", Leaf),
            ("timeout", Leaf),
            ("env", Map(Box::new(Leaf))),
        ]))
    };

    let active_hours = || {
        Struct(HashMap::from([
            ("start", Leaf),
            ("end", Leaf),
            ("timezone", Leaf),
        ]))
    };

    let qmd_collection = || Struct(HashMap::from([("paths", Leaf), ("globs", Leaf)]));

    let qmd = || {
        Struct(HashMap::from([
            ("command", Leaf),
            ("collections", Map(Box::new(qmd_collection()))),
            ("max_results", Leaf),
            ("timeout_ms", Leaf),
        ]))
    };

    Struct(HashMap::from([
        (
            "server",
            Struct(HashMap::from([
                ("bind", Leaf),
                ("port", Leaf),
                ("http_request_logs", Leaf),
                ("ws_request_logs", Leaf),
                ("update_repository_url", Leaf),
            ])),
        ),
        ("providers", MapWithFields {
            value: Box::new(provider_entry()),
            fields: HashMap::from([("offered", Array(Box::new(Leaf)))]),
        }),
        (
            "chat",
            Struct(HashMap::from([
                ("message_queue_mode", Leaf),
                ("priority_models", Leaf),
            ])),
        ),
        ("tools", tools()),
        (
            "skills",
            Struct(HashMap::from([
                ("enabled", Leaf),
                ("search_paths", Leaf),
                ("auto_load", Leaf),
            ])),
        ),
        (
            "mcp",
            Struct(HashMap::from([(
                "servers",
                Map(Box::new(mcp_server_entry())),
            )])),
        ),
        (
            "channels",
            Struct(HashMap::from([("telegram", Map(Box::new(Leaf)))])),
        ),
        (
            "tls",
            Struct(HashMap::from([
                ("enabled", Leaf),
                ("auto_generate", Leaf),
                ("cert_path", Leaf),
                ("key_path", Leaf),
                ("ca_cert_path", Leaf),
                ("http_redirect_port", Leaf),
            ])),
        ),
        ("auth", Struct(HashMap::from([("disabled", Leaf)]))),
        (
            "metrics",
            Struct(HashMap::from([
                ("enabled", Leaf),
                ("prometheus_endpoint", Leaf),
                ("labels", Map(Box::new(Leaf))),
            ])),
        ),
        (
            "identity",
            Struct(HashMap::from([
                ("name", Leaf),
                ("emoji", Leaf),
                ("creature", Leaf),
                ("vibe", Leaf),
            ])),
        ),
        (
            "user",
            Struct(HashMap::from([("name", Leaf), ("timezone", Leaf)])),
        ),
        (
            "hooks",
            Struct(HashMap::from([(
                "hooks",
                Array(Box::new(shell_hook_entry())),
            )])),
        ),
        (
            "memory",
            Struct(HashMap::from([
                ("backend", Leaf),
                ("provider", Leaf),
                ("base_url", Leaf),
                ("model", Leaf),
                ("api_key", Leaf),
                ("citations", Leaf),
                ("llm_reranking", Leaf),
                ("session_export", Leaf),
                ("qmd", qmd()),
            ])),
        ),
        (
            "tailscale",
            Struct(HashMap::from([("mode", Leaf), ("reset_on_exit", Leaf)])),
        ),
        (
            "failover",
            Struct(HashMap::from([
                ("enabled", Leaf),
                ("fallback_models", Leaf),
            ])),
        ),
        (
            "heartbeat",
            Struct(HashMap::from([
                ("enabled", Leaf),
                ("every", Leaf),
                ("model", Leaf),
                ("prompt", Leaf),
                ("ack_max_chars", Leaf),
                ("active_hours", active_hours()),
                ("sandbox_enabled", Leaf),
                ("sandbox_image", Leaf),
            ])),
        ),
        (
            "cron",
            Struct(HashMap::from([
                ("rate_limit_max", Leaf),
                ("rate_limit_window_secs", Leaf),
            ])),
        ),
    ]))
}

// ── Levenshtein distance ────────────────────────────────────────────────────

/// Compute the Levenshtein edit distance between two strings.
fn levenshtein(a: &str, b: &str) -> usize {
    let a_len = a.len();
    let b_len = b.len();
    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut prev: Vec<usize> = (0..=b_len).collect();
    let mut curr = vec![0; b_len + 1];

    for (i, ca) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.chars().enumerate() {
            let cost = if ca == cb {
                0
            } else {
                1
            };
            curr[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(curr[j] + 1);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b_len]
}

/// Find the best match for `needle` among `candidates` using Levenshtein
/// distance. Returns `Some(best)` if the distance is <= `max_distance`.
fn suggest<'a>(needle: &str, candidates: &[&'a str], max_distance: usize) -> Option<&'a str> {
    let mut best: Option<(&'a str, usize)> = None;
    for &candidate in candidates {
        let d = levenshtein(needle, candidate);
        if d > 0 && d <= max_distance && best.as_ref().is_none_or(|(_, bd)| d < *bd) {
            best = Some((candidate, d));
        }
    }
    best.map(|(s, _)| s)
}

// ── Core validation ─────────────────────────────────────────────────────────

/// Validate a config file at the given path, or discover the default config
/// file location if `path` is `None`.
#[must_use]
pub fn validate(path: Option<&Path>) -> ValidationResult {
    let config_path = if let Some(p) = path {
        Some(p.to_path_buf())
    } else {
        crate::loader::find_config_file()
    };

    let Some(ref actual_path) = config_path else {
        return ValidationResult {
            diagnostics: vec![Diagnostic {
                severity: Severity::Info,
                category: "file-ref",
                path: String::new(),
                message: "no config file found; using defaults".into(),
            }],
            config_path: None,
        };
    };

    match std::fs::read_to_string(actual_path) {
        Ok(content) => {
            let mut result = validate_toml_str(&content);
            result.config_path = Some(actual_path.clone());
            // File reference checks need actual paths resolved relative to config dir
            check_file_references(&content, actual_path, &mut result.diagnostics);
            result
        },
        Err(e) => ValidationResult {
            diagnostics: vec![Diagnostic {
                severity: Severity::Error,
                category: "syntax",
                path: String::new(),
                message: format!("failed to read config file: {e}"),
            }],
            config_path: Some(actual_path.clone()),
        },
    }
}

/// Validate a TOML string without file-system side effects (useful for tests
/// and the gateway).
#[must_use]
pub fn validate_toml_str(toml_str: &str) -> ValidationResult {
    let mut diagnostics = Vec::new();

    // 1. Syntax — parse raw TOML
    let toml_value: toml::Value = match toml::from_str(toml_str) {
        Ok(v) => v,
        Err(e) => {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                category: "syntax",
                path: String::new(),
                message: format!("TOML syntax error: {e}"),
            });
            return ValidationResult {
                diagnostics,
                config_path: None,
            };
        },
    };

    // 2. Unknown fields — walk the TOML tree against KnownKeys
    let schema = build_schema_map();
    check_unknown_fields(&toml_value, &schema, "", &mut diagnostics);

    // 3. Provider name hints
    if let Some(providers) = toml_value.get("providers").and_then(|v| v.as_table()) {
        check_provider_names(providers, &mut diagnostics);
    }

    // 4. Type check — attempt full deserialization
    if let Err(e) = toml::from_str::<MoltisConfig>(toml_str) {
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            category: "type-error",
            path: String::new(),
            message: format!("type error: {e}"),
        });
    }

    // 5. Semantic warnings on parsed config (only if it parses)
    if let Ok(config) = toml::from_str::<MoltisConfig>(toml_str) {
        check_semantic_warnings(&config, &mut diagnostics);
    }

    ValidationResult {
        diagnostics,
        config_path: None,
    }
}

/// Walk the TOML value tree against the schema tree and flag unknown keys.
fn check_unknown_fields(
    value: &toml::Value,
    schema: &KnownKeys,
    prefix: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match (value, schema) {
        (toml::Value::Table(table), KnownKeys::Struct(fields)) => {
            let known_keys: Vec<&str> = fields.keys().copied().collect();
            for (key, child_value) in table {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                if let Some(child_schema) = fields.get(key.as_str()) {
                    check_unknown_fields(child_value, child_schema, &path, diagnostics);
                } else {
                    let level = if prefix.is_empty() {
                        "at top level "
                    } else {
                        ""
                    };
                    let suggestion = suggest(key, &known_keys, 3);
                    let msg = if let Some(s) = suggestion {
                        format!("unknown field {level}(did you mean \"{s}\"?)")
                    } else {
                        format!("unknown field {level}")
                    };
                    diagnostics.push(Diagnostic {
                        severity: Severity::Error,
                        category: "unknown-field",
                        path,
                        message: msg.trim().to_string(),
                    });
                }
            }
        },
        (toml::Value::Table(table), KnownKeys::Map(value_schema)) => {
            for (key, child_value) in table {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                check_unknown_fields(child_value, value_schema, &path, diagnostics);
            }
        },
        (
            toml::Value::Table(table),
            KnownKeys::MapWithFields {
                value: value_schema,
                fields,
            },
        ) => {
            for (key, child_value) in table {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                if let Some(child_schema) = fields.get(key.as_str()) {
                    check_unknown_fields(child_value, child_schema, &path, diagnostics);
                } else {
                    check_unknown_fields(child_value, value_schema, &path, diagnostics);
                }
            }
        },
        (toml::Value::Array(arr), KnownKeys::Array(item_schema)) => {
            for (i, item) in arr.iter().enumerate() {
                let path = format!("{prefix}[{i}]");
                check_unknown_fields(item, item_schema, &path, diagnostics);
            }
        },
        // Leaf or type mismatch — stop recursion (type errors caught later)
        _ => {},
    }
}

/// Check provider names under `[providers]` and warn about unknown ones.
fn check_provider_names(
    providers: &toml::map::Map<String, toml::Value>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for name in providers.keys() {
        if PROVIDERS_META_KEYS.contains(&name.as_str()) {
            continue;
        }
        if !KNOWN_PROVIDER_NAMES.contains(&name.as_str()) {
            let suggestion = suggest(name, KNOWN_PROVIDER_NAMES, 3);
            let msg = if let Some(s) = suggestion {
                format!("unknown provider name (did you mean \"{s}\"?)")
            } else {
                "unknown provider name (custom providers are valid, but check for typos)".into()
            };
            diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                category: "unknown-provider",
                path: format!("providers.{name}"),
                message: msg,
            });
        }
    }
}

/// Run semantic checks on a successfully parsed config.
fn check_semantic_warnings(config: &MoltisConfig, diagnostics: &mut Vec<Diagnostic>) {
    let is_localhost = config.server.bind == "127.0.0.1"
        || config.server.bind == "localhost"
        || config.server.bind == "::1";

    // auth.disabled + non-localhost
    if config.auth.disabled && !is_localhost {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "security",
            path: "auth".into(),
            message: format!(
                "authentication is disabled while binding to {}",
                config.server.bind
            ),
        });
    }

    // TLS disabled + non-localhost
    if !config.tls.enabled && !is_localhost {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "security",
            path: "tls".into(),
            message: format!("TLS is disabled while binding to {}", config.server.bind),
        });
    }

    // TLS cert without key or vice versa
    let has_cert = config.tls.cert_path.is_some();
    let has_key = config.tls.key_path.is_some();
    if has_cert && !has_key {
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            category: "security",
            path: "tls".into(),
            message: "tls.cert_path is set but tls.key_path is missing".into(),
        });
    }
    if has_key && !has_cert {
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            category: "security",
            path: "tls".into(),
            message: "tls.key_path is set but tls.cert_path is missing".into(),
        });
    }

    // Sandbox mode off
    if config.tools.exec.sandbox.mode == "off" {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "security",
            path: "tools.exec.sandbox.mode".into(),
            message: "sandbox mode is disabled — commands run without isolation".into(),
        });
    }

    // Unknown tailscale mode
    let valid_ts_modes = ["off", "serve", "funnel"];
    if !valid_ts_modes.contains(&config.tailscale.mode.as_str()) {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "unknown-field",
            path: "tailscale.mode".into(),
            message: format!(
                "unknown tailscale mode \"{}\"; expected one of: {}",
                config.tailscale.mode,
                valid_ts_modes.join(", ")
            ),
        });
    }

    // Unknown sandbox backend
    let valid_sandbox_backends = ["auto", "docker", "apple-container"];
    if !valid_sandbox_backends.contains(&config.tools.exec.sandbox.backend.as_str()) {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "unknown-field",
            path: "tools.exec.sandbox.backend".into(),
            message: format!(
                "unknown sandbox backend \"{}\"; expected one of: {}",
                config.tools.exec.sandbox.backend,
                valid_sandbox_backends.join(", ")
            ),
        });
    }

    // Unknown memory backend
    if let Some(ref backend) = config.memory.backend {
        let valid_backends = ["builtin", "qmd"];
        if !valid_backends.contains(&backend.as_str()) {
            diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                category: "unknown-field",
                path: "memory.backend".into(),
                message: format!(
                    "unknown memory backend \"{backend}\"; expected one of: {}",
                    valid_backends.join(", ")
                ),
            });
        }
    }

    // Unknown memory provider
    if let Some(ref provider) = config.memory.provider {
        let valid_providers = ["local", "ollama", "openai", "custom"];
        if !valid_providers.contains(&provider.as_str()) {
            diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                category: "unknown-field",
                path: "memory.provider".into(),
                message: format!(
                    "unknown memory provider \"{provider}\"; expected one of: {}",
                    valid_providers.join(", ")
                ),
            });
        }
    }

    // Unknown exec security level
    let valid_security_levels = ["allowlist", "permissive", "strict"];
    if !valid_security_levels.contains(&config.tools.exec.security_level.as_str()) {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            category: "unknown-field",
            path: "tools.exec.security_level".into(),
            message: format!(
                "unknown security level \"{}\"; expected one of: {}",
                config.tools.exec.security_level,
                valid_security_levels.join(", ")
            ),
        });
    }

    // port == 0
    if config.server.port == 0 {
        diagnostics.push(Diagnostic {
            severity: Severity::Info,
            category: "security",
            path: "server.port".into(),
            message: "port is 0; a random port will be assigned at startup".into(),
        });
    }
}

/// Check that file paths referenced in TLS config exist on disk.
fn check_file_references(toml_str: &str, _config_path: &Path, diagnostics: &mut Vec<Diagnostic>) {
    // Only check if we can parse the config
    let Ok(config) = toml::from_str::<MoltisConfig>(toml_str) else {
        return;
    };

    let file_refs: &[(&str, &Option<String>)] = &[
        ("tls.cert_path", &config.tls.cert_path),
        ("tls.key_path", &config.tls.key_path),
        ("tls.ca_cert_path", &config.tls.ca_cert_path),
    ];

    for (path_name, value) in file_refs {
        if let Some(file_path) = value {
            let p = Path::new(file_path);
            if !p.exists() {
                diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    category: "file-ref",
                    path: (*path_name).into(),
                    message: format!("file not found: {file_path}"),
                });
            }
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_identical() {
        assert_eq!(levenshtein("hello", "hello"), 0);
    }

    #[test]
    fn levenshtein_empty() {
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("", ""), 0);
    }

    #[test]
    fn levenshtein_single_edit() {
        assert_eq!(levenshtein("server", "sever"), 1); // deletion
        assert_eq!(levenshtein("bind", "bnd"), 1); // deletion
        assert_eq!(levenshtein("port", "prt"), 1); // deletion
    }

    #[test]
    fn levenshtein_substitution() {
        assert_eq!(levenshtein("cat", "car"), 1);
        assert_eq!(levenshtein("anthropic", "anthrpic"), 1);
    }

    #[test]
    fn levenshtein_insertion() {
        assert_eq!(levenshtein("serer", "server"), 1);
    }

    #[test]
    fn unknown_top_level_key_with_suggestion() {
        let result = validate_toml_str("sever = 42\n");
        let unknown = result
            .diagnostics
            .iter()
            .find(|d| d.category == "unknown-field" && d.path == "sever");
        assert!(
            unknown.is_some(),
            "expected unknown-field diagnostic for 'sever'"
        );
        let d = unknown.unwrap();
        assert_eq!(d.severity, Severity::Error);
        assert!(
            d.message.contains("server"),
            "expected suggestion 'server' in message: {}",
            d.message
        );
    }

    #[test]
    fn unknown_nested_key_with_suggestion() {
        let toml = r#"
[server]
bnd = "0.0.0.0"
"#;
        let result = validate_toml_str(toml);
        let unknown = result
            .diagnostics
            .iter()
            .find(|d| d.category == "unknown-field" && d.path == "server.bnd");
        assert!(
            unknown.is_some(),
            "expected unknown-field for 'server.bnd', got: {:?}",
            result.diagnostics
        );
        assert!(unknown.unwrap().message.contains("bind"));
    }

    #[test]
    fn unknown_field_inside_provider_entry() {
        let toml = r#"
[providers.anthropic]
api_ky = "sk-test"
"#;
        let result = validate_toml_str(toml);
        let unknown = result
            .diagnostics
            .iter()
            .find(|d| d.category == "unknown-field" && d.path == "providers.anthropic.api_ky");
        assert!(
            unknown.is_some(),
            "expected unknown-field for 'providers.anthropic.api_ky', got: {:?}",
            result.diagnostics
        );
        assert!(unknown.unwrap().message.contains("api_key"));
    }

    #[test]
    fn misspelled_provider_name_warned_with_suggestion() {
        let toml = r#"
[providers.anthrpic]
enabled = true
"#;
        let result = validate_toml_str(toml);
        let warning = result
            .diagnostics
            .iter()
            .find(|d| d.category == "unknown-provider" && d.path == "providers.anthrpic");
        assert!(
            warning.is_some(),
            "expected unknown-provider for 'anthrpic', got: {:?}",
            result.diagnostics
        );
        let d = warning.unwrap();
        assert_eq!(d.severity, Severity::Warning);
        assert!(d.message.contains("anthropic"));
    }

    #[test]
    fn providers_offered_key_not_treated_as_provider_name() {
        let toml = r#"
[providers]
offered = ["openai", "github-copilot"]
"#;
        let result = validate_toml_str(toml);
        let offered_warning = result
            .diagnostics
            .iter()
            .find(|d| d.category == "unknown-provider" && d.path == "providers.offered");
        assert!(
            offered_warning.is_none(),
            "providers.offered should be treated as metadata, got: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn custom_provider_name_warned_without_close_match() {
        let toml = r#"
[providers.my_custom_llm]
enabled = true
"#;
        let result = validate_toml_str(toml);
        let warning = result
            .diagnostics
            .iter()
            .find(|d| d.category == "unknown-provider");
        assert!(warning.is_some());
        let d = warning.unwrap();
        assert_eq!(d.severity, Severity::Warning);
        assert!(d.message.contains("custom providers are valid"));
    }

    #[test]
    fn empty_config_is_valid() {
        let result = validate_toml_str("");
        assert!(
            !result.has_errors(),
            "empty config should be valid, got: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn full_valid_config_no_diagnostics() {
        let toml = r#"
[server]
bind = "127.0.0.1"
port = 8080

[providers.anthropic]
enabled = true
model = "claude-sonnet-4-20250514"

[auth]
disabled = false

[tls]
enabled = true
auto_generate = true

[tools.exec]
default_timeout_secs = 30

[tools.exec.sandbox]
mode = "all"
backend = "auto"

[tailscale]
mode = "off"

[memory]
backend = "builtin"
provider = "local"

[metrics]
enabled = true

[failover]
enabled = true

[heartbeat]
enabled = true
every = "30m"

[heartbeat.active_hours]
start = "08:00"
end = "24:00"

[cron]
rate_limit_max = 10
"#;
        let result = validate_toml_str(toml);
        let errors: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "expected no errors for valid config, got: {errors:?}"
        );
    }

    #[test]
    fn syntax_error_detected() {
        let result = validate_toml_str("this is not valid toml [[[");
        assert!(result.has_errors());
        let syntax = result.diagnostics.iter().find(|d| d.category == "syntax");
        assert!(syntax.is_some());
    }

    #[test]
    fn auth_disabled_non_localhost_warned() {
        let toml = r#"
[server]
bind = "0.0.0.0"

[auth]
disabled = true
"#;
        let result = validate_toml_str(toml);
        let warning = result
            .diagnostics
            .iter()
            .find(|d| d.category == "security" && d.path == "auth");
        assert!(
            warning.is_some(),
            "expected security warning for auth disabled + non-localhost"
        );
    }

    #[test]
    fn auth_disabled_localhost_not_warned() {
        let toml = r#"
[server]
bind = "127.0.0.1"

[auth]
disabled = true
"#;
        let result = validate_toml_str(toml);
        let warning = result
            .diagnostics
            .iter()
            .find(|d| d.category == "security" && d.path == "auth");
        assert!(
            warning.is_none(),
            "should not warn about auth disabled on localhost"
        );
    }

    #[test]
    fn tls_cert_without_key_is_error() {
        let toml = r#"
[tls]
cert_path = "/path/to/cert.pem"
"#;
        let result = validate_toml_str(toml);
        let error = result.diagnostics.iter().find(|d| {
            d.severity == Severity::Error && d.path == "tls" && d.message.contains("key_path")
        });
        assert!(
            error.is_some(),
            "expected error for cert_path without key_path, got: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn tls_key_without_cert_is_error() {
        let toml = r#"
[tls]
key_path = "/path/to/key.pem"
"#;
        let result = validate_toml_str(toml);
        let error = result.diagnostics.iter().find(|d| {
            d.severity == Severity::Error && d.path == "tls" && d.message.contains("cert_path")
        });
        assert!(
            error.is_some(),
            "expected error for key_path without cert_path"
        );
    }

    #[test]
    fn unknown_tailscale_mode_warned() {
        let toml = r#"
[tailscale]
mode = "tunnel"
"#;
        let result = validate_toml_str(toml);
        let warning = result
            .diagnostics
            .iter()
            .find(|d| d.path == "tailscale.mode");
        assert!(
            warning.is_some(),
            "expected warning for unknown tailscale mode 'tunnel'"
        );
    }

    #[test]
    fn sandbox_mode_off_warned() {
        let toml = r#"
[tools.exec.sandbox]
mode = "off"
"#;
        let result = validate_toml_str(toml);
        let warning = result
            .diagnostics
            .iter()
            .find(|d| d.path == "tools.exec.sandbox.mode");
        assert!(warning.is_some(), "expected warning for sandbox mode off");
    }

    #[test]
    fn port_zero_info() {
        let toml = r#"
[server]
port = 0
"#;
        let result = validate_toml_str(toml);
        let info = result
            .diagnostics
            .iter()
            .find(|d| d.severity == Severity::Info && d.path == "server.port");
        assert!(info.is_some(), "expected info for port 0");
    }

    #[test]
    fn unknown_memory_backend_warned() {
        let toml = r#"
[memory]
backend = "postgres"
"#;
        let result = validate_toml_str(toml);
        let warning = result
            .diagnostics
            .iter()
            .find(|d| d.path == "memory.backend");
        assert!(
            warning.is_some(),
            "expected warning for unknown memory backend"
        );
    }

    #[test]
    fn unknown_memory_provider_warned() {
        let toml = r#"
[memory]
provider = "pinecone"
"#;
        let result = validate_toml_str(toml);
        let warning = result
            .diagnostics
            .iter()
            .find(|d| d.path == "memory.provider");
        assert!(
            warning.is_some(),
            "expected warning for unknown memory provider"
        );
    }

    #[test]
    fn unknown_sandbox_backend_warned() {
        let toml = r#"
[tools.exec.sandbox]
backend = "podman"
"#;
        let result = validate_toml_str(toml);
        let warning = result
            .diagnostics
            .iter()
            .find(|d| d.path == "tools.exec.sandbox.backend");
        assert!(
            warning.is_some(),
            "expected warning for unknown sandbox backend"
        );
    }

    #[test]
    fn unknown_security_level_warned() {
        let toml = r#"
[tools.exec]
security_level = "paranoid"
"#;
        let result = validate_toml_str(toml);
        let warning = result
            .diagnostics
            .iter()
            .find(|d| d.path == "tools.exec.security_level");
        assert!(
            warning.is_some(),
            "expected warning for unknown security level"
        );
    }

    #[test]
    fn tls_disabled_non_localhost_warned() {
        let toml = r#"
[server]
bind = "0.0.0.0"

[tls]
enabled = false
"#;
        let result = validate_toml_str(toml);
        let warning = result
            .diagnostics
            .iter()
            .find(|d| d.category == "security" && d.path == "tls");
        assert!(
            warning.is_some(),
            "expected security warning for TLS disabled + non-localhost"
        );
    }

    #[test]
    fn mcp_server_entries_validated() {
        let toml = r#"
[mcp.servers.myserver]
command = "node"
args = ["server.js"]
enabled = true
transport = "stdio"
unknwon_field = true
"#;
        let result = validate_toml_str(toml);
        let unknown = result
            .diagnostics
            .iter()
            .find(|d| d.category == "unknown-field" && d.path.contains("myserver"));
        assert!(
            unknown.is_some(),
            "expected unknown-field in MCP server entry, got: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn hooks_array_entries_validated() {
        let toml = r#"
[[hooks.hooks]]
name = "test"
command = "echo test"
events = ["startup"]
unknwon = "value"
"#;
        let result = validate_toml_str(toml);
        let unknown = result
            .diagnostics
            .iter()
            .find(|d| d.category == "unknown-field" && d.path.contains("hooks.hooks"));
        assert!(
            unknown.is_some(),
            "expected unknown-field in hooks entry, got: {:?}",
            result.diagnostics
        );
    }

    /// Schema drift guard: verify every key from `MoltisConfig::default()` is
    /// represented in `build_schema_map()`.
    #[test]
    fn schema_drift_guard() {
        let config = MoltisConfig::default();
        let toml_value = toml::Value::try_from(&config).expect("serialize default config");
        let schema = build_schema_map();
        let mut missing = Vec::new();
        collect_missing_keys(&toml_value, &schema, "", &mut missing);
        assert!(
            missing.is_empty(),
            "schema map is missing keys present in MoltisConfig::default(): {missing:?}\n\
             Update build_schema_map() in validate.rs to include these fields."
        );
    }

    /// Helper for schema drift guard: recursively collect keys in `value` that
    /// are not present in `schema`.
    fn collect_missing_keys(
        value: &toml::Value,
        schema: &KnownKeys,
        prefix: &str,
        missing: &mut Vec<String>,
    ) {
        match (value, schema) {
            (toml::Value::Table(table), KnownKeys::Struct(fields)) => {
                for (key, child_value) in table {
                    let path = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    if let Some(child_schema) = fields.get(key.as_str()) {
                        collect_missing_keys(child_value, child_schema, &path, missing);
                    } else {
                        missing.push(path);
                    }
                }
            },
            (toml::Value::Table(table), KnownKeys::Map(value_schema)) => {
                for (key, child_value) in table {
                    let path = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    collect_missing_keys(child_value, value_schema, &path, missing);
                }
            },
            (
                toml::Value::Table(table),
                KnownKeys::MapWithFields {
                    value: value_schema,
                    fields,
                },
            ) => {
                for (key, child_value) in table {
                    let path = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    if let Some(child_schema) = fields.get(key.as_str()) {
                        collect_missing_keys(child_value, child_schema, &path, missing);
                    } else {
                        collect_missing_keys(child_value, value_schema, &path, missing);
                    }
                }
            },
            (toml::Value::Array(arr), KnownKeys::Array(item_schema)) => {
                for (i, item) in arr.iter().enumerate() {
                    let path = format!("{prefix}[{i}]");
                    collect_missing_keys(item, item_schema, &path, missing);
                }
            },
            _ => {},
        }
    }

    #[test]
    fn suggest_finds_close_match() {
        let candidates = &["server", "providers", "auth", "tls"];
        assert_eq!(suggest("sever", candidates, 3), Some("server"));
        assert_eq!(suggest("servar", candidates, 3), Some("server"));
        assert_eq!(suggest("provders", candidates, 3), Some("providers"));
    }

    #[test]
    fn suggest_returns_none_for_distant() {
        let candidates = &["server", "providers", "auth", "tls"];
        assert_eq!(suggest("xxxxxxxxx", candidates, 3), None);
    }

    #[test]
    fn valid_known_providers_not_warned() {
        let toml = r#"
[providers.anthropic]
enabled = true

[providers.openai]
enabled = true

[providers.ollama]
enabled = true
"#;
        let result = validate_toml_str(toml);
        let warnings: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.category == "unknown-provider")
            .collect();
        assert!(
            warnings.is_empty(),
            "known providers should not be warned about: {warnings:?}"
        );
    }
}
