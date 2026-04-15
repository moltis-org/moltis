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
models = ["claude-sonnet-4-20250514"]

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

fn server_terminal_enabled_is_known_field() {
    let toml = r#"
[server]
terminal_enabled = false
"#;
    let result = validate_toml_str(toml);
    let unknown = result
        .diagnostics
        .iter()
        .find(|d| d.category == "unknown-field" && d.path.contains("terminal_enabled"));
    assert!(
        unknown.is_none(),
        "terminal_enabled should be a known field, got: {unknown:?}"
    );
}
