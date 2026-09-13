use super::*;

#[test]
fn chat_reasoning_default_valid_values_are_recognized() {
    for value in [
        "minimal",
        "low",
        "medium",
        "high",
        "xhigh",
        "extra-high",
        "max",
    ] {
        let result = validate_toml_str(&format!("[chat]\nreasoning_default = {value:?}"));
        assert!(
            !result
                .diagnostics
                .iter()
                .any(|d| { d.severity == Severity::Error || d.category == "unknown-field" }),
            "value={value}: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn chat_reasoning_default_invalid_values_report_type_error() {
    for value in ["\"off\"", "\"extreme\"", "\"\"", "42", "true", "[]", "{}"] {
        let result = validate_toml_str(&format!("[chat]\nreasoning_default = {value}"));
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| { d.severity == Severity::Error && d.category == "type-error" }),
            "value={value}: {:?}",
            result.diagnostics
        );
    }
}
