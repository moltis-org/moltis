use super::*;

#[test]
fn unknown_memory_backend_is_parse_error() {
    let toml = r#"
[memory]
backend = "postgres"
"#;
    let result = validate_toml_str(toml);
    assert!(
        result.has_errors(),
        "expected parse error for unknown memory backend"
    );
}

#[test]
fn unknown_memory_citations_mode_is_parse_error() {
    let toml = r#"
[memory]
citations = "sometimes"
"#;
    let result = validate_toml_str(toml);
    assert!(
        result.has_errors(),
        "expected parse error for unknown memory citations mode"
    );
}

#[test]
fn unknown_memory_search_merge_strategy_is_parse_error() {
    let toml = r#"
[memory]
search_merge_strategy = "blend"
"#;
    let result = validate_toml_str(toml);
    assert!(
        result.has_errors(),
        "expected parse error for unknown memory search merge strategy"
    );
}

#[test]
fn unknown_memory_provider_is_parse_error() {
    let toml = r#"
[memory]
provider = "pinecone"
"#;
    let result = validate_toml_str(toml);
    assert!(
        result.has_errors(),
        "expected parse error for unknown memory provider"
    );
}

#[test]
fn memory_disable_rag_is_valid_field() {
    let toml = r#"
[memory]
disable_rag = true
"#;
    let result = validate_toml_str(toml);
    let unknown = result
        .diagnostics
        .iter()
        .find(|d| d.category == "unknown-field" && d.path == "memory.disable_rag");
    assert!(
        unknown.is_none(),
        "memory.disable_rag should be accepted as a known field"
    );
}

#[test]
fn memory_style_is_valid_field() {
    let toml = r#"
[memory]
style = "search-only"
"#;
    let result = validate_toml_str(toml);
    let unknown = result
        .diagnostics
        .iter()
        .find(|d| d.category == "unknown-field" && d.path == "memory.style");
    assert!(
        unknown.is_none(),
        "memory.style should be accepted as a known field"
    );
}

#[test]
fn memory_agent_write_mode_is_valid_field() {
    let toml = r#"
[memory]
agent_write_mode = "prompt-only"
"#;
    let result = validate_toml_str(toml);
    let unknown = result
        .diagnostics
        .iter()
        .find(|d| d.category == "unknown-field" && d.path == "memory.agent_write_mode");
    assert!(
        unknown.is_none(),
        "memory.agent_write_mode should be accepted as a known field"
    );
}

#[test]
fn memory_user_profile_write_mode_is_valid_field() {
    let toml = r#"
[memory]
user_profile_write_mode = "explicit-only"
"#;
    let result = validate_toml_str(toml);
    let unknown = result
        .diagnostics
        .iter()
        .find(|d| d.category == "unknown-field" && d.path == "memory.user_profile_write_mode");
    assert!(
        unknown.is_none(),
        "memory.user_profile_write_mode should be accepted as a known field"
    );
}

#[test]
fn legacy_memory_embedding_fields_warn_but_do_not_error() {
    let toml = r#"
[memory]
embedding_provider = "custom"
embedding_model = "intfloat/multilingual-e5-small"
embedding_base_url = "http://moltis-embeddings:7997/v1"
embedding_dimensions = 384
"#;
    let result = validate_toml_str(toml);

    let unknown: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.category == "unknown-field" && d.path.starts_with("memory.embedding_"))
        .collect();
    assert!(
        unknown.is_empty(),
        "legacy embedding fields should not be unknown: {:?}",
        result.diagnostics
    );

    let deprecated: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.category == "deprecated-field")
        .collect();
    assert_eq!(
        deprecated.len(),
        4,
        "expected deprecation warnings for all legacy fields: {:?}",
        result.diagnostics
    );
    assert!(
        deprecated
            .iter()
            .any(|d| d.path == "memory.embedding_provider"
                && d.message.contains("memory.provider")),
        "expected replacement warning for embedding_provider"
    );
    assert!(
        deprecated
            .iter()
            .any(|d| d.path == "memory.embedding_base_url"
                && d.message.contains("memory.base_url")),
        "expected replacement warning for embedding_base_url"
    );
    assert!(
        deprecated
            .iter()
            .any(|d| d.path == "memory.embedding_model" && d.message.contains("memory.model")),
        "expected replacement warning for embedding_model"
    );
    assert!(
        deprecated
            .iter()
            .any(|d| d.path == "memory.embedding_dimensions" && d.message.contains("ignored")),
        "expected ignored warning for embedding_dimensions"
    );
    assert!(
        !result.has_errors(),
        "legacy embedding fields should remain usable: {:?}",
        result.diagnostics
    );
}

#[test]
fn conflicting_legacy_and_modern_memory_field_reports_targeted_error() {
    let toml = r#"
[memory]
provider = "custom"
embedding_provider = "custom"
"#;
    let result = validate_toml_str(toml);

    let conflict = result
        .diagnostics
        .iter()
        .find(|d| {
            d.category == "deprecated-field"
                && d.severity == Severity::Error
                && d.path == "memory.embedding_provider"
        })
        .unwrap_or_else(|| panic!("expected targeted conflict error: {:?}", result.diagnostics));
    assert!(
        conflict
            .message
            .contains("remove \"memory.embedding_provider\""),
        "expected removal guidance, got: {}",
        conflict.message
    );

    let type_error = result
        .diagnostics
        .iter()
        .find(|d| d.category == "type-error");
    assert!(
        type_error.is_none(),
        "expected duplicate-field type error to be suppressed: {:?}",
        result.diagnostics
    );
}

#[test]
fn memory_prefetch_fields_are_valid() {
    let toml = r#"
[memory]
enable_prefetch = true
prefetch_limit = 5
auto_extract_interval = 10
enable_session_summary = false
"#;
    let result = validate_toml_str(toml);
    let unknown: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.category == "unknown-field" && d.path.starts_with("memory."))
        .collect();
    assert!(
        unknown.is_empty(),
        "new memory lifecycle fields should be accepted: {unknown:?}"
    );
    assert!(
        !result.has_errors(),
        "no errors for valid memory lifecycle config: {:?}",
        result.diagnostics
    );
}

#[test]
fn skills_enable_self_improvement_is_valid() {
    let toml = r#"
[skills]
enable_self_improvement = false
"#;
    let result = validate_toml_str(toml);
    let unknown = result
        .diagnostics
        .iter()
        .find(|d| d.category == "unknown-field" && d.path == "skills.enable_self_improvement");
    assert!(
        unknown.is_none(),
        "skills.enable_self_improvement should be accepted"
    );
}

#[test]
fn memory_lifecycle_fields_default_to_true() {
    let config: MoltisConfig = toml::from_str("").unwrap();
    assert!(
        config.memory.enable_prefetch,
        "enable_prefetch should default true"
    );
    assert_eq!(
        config.memory.prefetch_limit, 3,
        "prefetch_limit should default 3"
    );
    assert_eq!(
        config.memory.auto_extract_interval, 5,
        "auto_extract_interval should default 5"
    );
    assert!(
        config.memory.enable_session_summary,
        "enable_session_summary should default true"
    );
    assert!(
        config.skills.enable_self_improvement,
        "enable_self_improvement should default true"
    );
}

#[test]
fn duplicate_field_suppression_matches_only_conflicting_replacements() {
    assert!(should_suppress_deprecated_conflict_type_error(
        "type error: duplicate field `provider`",
        &["provider"]
    ));
    assert!(!should_suppress_deprecated_conflict_type_error(
        "type error: duplicate field `base_url`",
        &["provider"]
    ));
}
