use std::collections::BTreeSet;

use super::*;

fn diagnostics_for(toml: &str, severity: Severity) -> BTreeSet<String> {
    validate_toml_str(toml)
        .diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == severity)
        .map(|diagnostic| diagnostic.path)
        .filter(|path| path.starts_with("instrumentation."))
        .collect()
}

#[test]
fn sample_rate_must_be_finite_and_in_range() {
    for sample_rate in ["nan", "inf", "-inf", "-0.01", "1.01"] {
        let result =
            validate_toml_str(&format!("[instrumentation]\nsample_rate = {sample_rate}\n"));
        let diagnostic = result.diagnostics.iter().find(|diagnostic| {
            diagnostic.severity == Severity::Error
                && diagnostic.path == "instrumentation.sample_rate"
        });

        assert!(
            diagnostic.is_some(),
            "sample_rate {sample_rate} should be rejected: {:?}",
            result.diagnostics
        );
    }

    for sample_rate in ["0.0", "0.25", "1.0"] {
        let errors = diagnostics_for(
            &format!("[instrumentation]\nsample_rate = {sample_rate}\n"),
            Severity::Error,
        );
        assert!(
            !errors.contains("instrumentation.sample_rate"),
            "sample_rate {sample_rate} should be accepted"
        );
    }
}

#[test]
fn batching_and_backend_timeouts_must_be_nonzero() {
    let errors = diagnostics_for(
        r#"
[instrumentation]
queue_capacity = 0
flush_interval_ms = 0
max_batch_bytes = 0

[instrumentation.langfuse]
timeout_secs = 0

[instrumentation.otlp]
timeout_secs = 0

[instrumentation.datadog]
timeout_secs = 0

[instrumentation.feedback]
link_retention_days = 0
"#,
        Severity::Error,
    );

    assert_eq!(
        errors,
        BTreeSet::from([
            "instrumentation.datadog.timeout_secs".to_string(),
            "instrumentation.flush_interval_ms".to_string(),
            "instrumentation.feedback.link_retention_days".to_string(),
            "instrumentation.langfuse.timeout_secs".to_string(),
            "instrumentation.max_batch_bytes".to_string(),
            "instrumentation.otlp.timeout_secs".to_string(),
            "instrumentation.queue_capacity".to_string(),
        ])
    );
}

#[test]
fn operationally_extreme_delivery_settings_are_warned() {
    let warnings = diagnostics_for(
        r#"
[instrumentation]
queue_capacity = 1000001
flush_interval_ms = 300001
max_batch_bytes = 100000001

[instrumentation.langfuse]
timeout_secs = 301

[instrumentation.otlp]
timeout_secs = 301

[instrumentation.datadog]
timeout_secs = 301
"#,
        Severity::Warning,
    );

    assert_eq!(warnings.len(), 6, "unexpected warnings: {warnings:?}");
}
