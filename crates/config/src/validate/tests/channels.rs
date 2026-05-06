use super::*;

#[test]
fn channels_offered_accepted_without_warning() {
    let toml = r#"
[channels]
offered = ["telegram"]
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.path.starts_with("channels.offered"));
    assert!(
        warning.is_none(),
        "valid channels.offered should not produce warnings, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn channels_offered_discord_accepted() {
    let toml = r#"
[channels]
offered = ["telegram", "discord"]
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.path.starts_with("channels.offered") && d.category == "unknown-field");
    assert!(
        warning.is_none(),
        "discord in channels.offered should not produce warnings, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn channels_discord_config_accepted() {
    let toml = r#"
[channels.discord.my_bot]
token = "test-token"
dm_policy = "allowlist"
"#;
    let result = validate_toml_str(toml);
    let error = result
        .diagnostics
        .iter()
        .find(|d| d.path.starts_with("channels.discord") && d.severity == Severity::Error);
    assert!(
        error.is_none(),
        "discord channel config should be accepted, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn channels_offered_unknown_type_warned() {
    let toml = r#"
[channels]
offered = ["telegram", "foobar"]
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.path == "channels.offered[1]" && d.category == "unknown-field");
    assert!(
        warning.is_some(),
        "unknown channel type should produce warning, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn channels_offered_slack_accepted() {
    let toml = r#"
[channels]
offered = ["telegram", "slack"]
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.path == "channels.offered[1]" && d.category == "unknown-field");
    assert!(
        warning.is_none(),
        "slack should be accepted, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn channels_offered_matrix_accepted() {
    let toml = r#"
[channels]
offered = ["telegram", "matrix"]
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.path == "channels.offered[1]" && d.category == "unknown-field");
    assert!(
        warning.is_none(),
        "matrix should be accepted, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn channels_offered_dynamic_type_accepted() {
    let toml = r#"
[channels]
offered = ["telegram", "slack"]

[channels.slack.my-bot]
token = "xoxb-test"
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.path.starts_with("channels.offered") && d.category == "unknown-field");
    assert!(
        warning.is_none(),
        "dynamically configured channel type should be accepted in offered, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn channels_extra_config_accepted() {
    let toml = r#"
[channels.slack.my-bot]
token = "xoxb-test"
dm_policy = "allowlist"
"#;
    let result = validate_toml_str(toml);
    let error = result.diagnostics.iter().find(|d| {
        d.path.starts_with("channels.slack")
            && (d.severity == Severity::Error || d.category == "unknown-field")
    });
    assert!(
        error.is_none(),
        "extra channel config should be accepted without errors, got: {:?}",
        result.diagnostics
    );
}
