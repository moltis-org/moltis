use super::*;

#[test]
fn reasoning_effort_valid_values_no_error() {
    for effort in &["minimal", "low", "medium", "high", "xhigh"] {
        let toml = format!(
            r#"
            [agents.presets.thinker]
            model = "claude-opus-4-5-20251101"
            reasoning_effort = "{effort}"
            "#
        );
        let result = validate_toml_str(&toml);
        let errors: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.path.contains("reasoning_effort") && d.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "effort={effort} should be valid: {errors:?}"
        );
    }
}

#[test]
fn reasoning_effort_invalid_value_reports_type_error() {
    let toml = r#"
    [agents.presets.thinker]
    model = "claude-opus-4-5-20251101"
    reasoning_effort = "extreme"
    "#;
    let result = validate_toml_str(toml);
    let error = result
        .diagnostics
        .iter()
        .find(|d| d.category == "type-error" && d.severity == Severity::Error);
    assert!(
        error.is_some(),
        "invalid reasoning_effort should produce type error: {:?}",
        result.diagnostics
    );
}

#[test]
fn reasoning_effort_recognized_in_schema() {
    let toml = r#"
    [agents.presets.thinker]
    reasoning_effort = "high"
    "#;
    let result = validate_toml_str(toml);
    let unknown = result
        .diagnostics
        .iter()
        .find(|d| d.category == "unknown-field" && d.message.contains("reasoning_effort"));
    assert!(
        unknown.is_none(),
        "reasoning_effort should be a recognized field, got: {:?}",
        result.diagnostics
    );
}

fn find_preset_silent_policy_warning(result: &ValidationResult) -> Option<&Diagnostic> {
    result.diagnostics.iter().find(|d| {
        d.category == "security" && d.path == "agents.presets" && d.message.contains("spawn_agent")
    })
}

#[test]
fn preset_tools_deny_without_main_policy_warns() {
    let toml = r#"
[agents]
default_preset = "full"

[agents.presets.full]
[agents.presets.full.tools]
deny = ["browser", "web_fetch"]
"#;
    let result = validate_toml_str(toml);
    let warning = find_preset_silent_policy_warning(&result).unwrap_or_else(|| {
        panic!(
            "expected silent-policy warning, got: {:?}",
            result.diagnostics
        )
    });
    assert_eq!(warning.severity, Severity::Warning);
    assert!(
        warning.message.contains("\"full\""),
        "expected preset name in message: {}",
        warning.message
    );
    assert!(
        warning.message.contains("[tools.policy]"),
        "expected pointer to [tools.policy] in message: {}",
        warning.message
    );
}

#[test]
fn preset_tools_allow_without_main_policy_also_warns() {
    let toml = r#"
[agents.presets.research]
[agents.presets.research.tools]
allow = ["web_search", "web_fetch"]
"#;
    let result = validate_toml_str(toml);
    let warning = find_preset_silent_policy_warning(&result).unwrap_or_else(|| {
        panic!(
            "expected silent-policy warning, got: {:?}",
            result.diagnostics
        )
    });
    assert!(warning.message.contains("\"research\""));
}

#[test]
fn preset_tools_deny_with_main_policy_deny_does_not_warn() {
    let toml = r#"
[tools.policy]
deny = ["exec"]

[agents.presets.full]
[agents.presets.full.tools]
deny = ["browser"]
"#;
    let result = validate_toml_str(toml);
    assert!(
        find_preset_silent_policy_warning(&result).is_none(),
        "should not warn when [tools.policy] is non-empty, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn preset_tools_deny_with_main_policy_allow_does_not_warn() {
    let toml = r#"
[tools.policy]
allow = ["web_search"]

[agents.presets.full]
[agents.presets.full.tools]
deny = ["browser"]
"#;
    let result = validate_toml_str(toml);
    assert!(
        find_preset_silent_policy_warning(&result).is_none(),
        "should not warn when [tools.policy] has allow list, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn preset_tools_deny_with_main_policy_profile_does_not_warn() {
    let toml = r#"
[tools.policy]
profile = "default"

[agents.presets.full]
[agents.presets.full.tools]
deny = ["browser"]
"#;
    let result = validate_toml_str(toml);
    assert!(
        find_preset_silent_policy_warning(&result).is_none(),
        "should not warn when [tools.policy.profile] is set, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn empty_preset_tools_does_not_warn() {
    let toml = r#"
[agents]
default_preset = "basic"

[agents.presets.basic]
model = "openai/gpt-5.2"
"#;
    let result = validate_toml_str(toml);
    assert!(
        find_preset_silent_policy_warning(&result).is_none(),
        "should not warn when presets declare no tool policy, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn multiple_offending_presets_are_rolled_up() {
    let toml = r#"
[agents.presets.full]
[agents.presets.full.tools]
deny = ["browser"]

[agents.presets.minimal]
[agents.presets.minimal.tools]
allow = ["web_search"]
"#;
    let result = validate_toml_str(toml);
    let warning = find_preset_silent_policy_warning(&result).unwrap_or_else(|| {
        panic!(
            "expected silent-policy warning, got: {:?}",
            result.diagnostics
        )
    });
    assert!(
        warning.message.contains("\"full\"") && warning.message.contains("\"minimal\""),
        "expected both preset names in single rolled-up warning: {}",
        warning.message
    );
    // And only one such diagnostic should be emitted.
    let count = result
        .diagnostics
        .iter()
        .filter(|d| d.category == "security" && d.path == "agents.presets")
        .count();
    assert_eq!(count, 1, "expected exactly one rolled-up warning");
}
