//! Configuration validation engine.
//!
//! Validates TOML configuration files against the known schema, detects
//! unknown/misspelled fields, and reports security warnings.

use std::path::Path;

use crate::schema::MoltisConfig;

#[path = "validate/schema_map.rs"]
mod schema_map;
#[path = "validate/semantic.rs"]
mod semantic;

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
    /// Category: "syntax", "unknown-field", "deprecated-field", "unknown-provider", "type-error",
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
            semantic::check_file_references(&content, actual_path, &mut result.diagnostics);
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

    // 1. Syntax - parse raw TOML
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

    // 2. Unknown fields - walk the TOML tree against KnownKeys
    let schema = schema_map::build_schema_map();
    schema_map::check_unknown_fields(&toml_value, &schema, "", &mut diagnostics);

    // 3. Deprecation warnings on raw TOML keys
    let conflicting_replacements = semantic::check_deprecated_fields(&toml_value, &mut diagnostics);

    // 4. Provider name hints
    if let Some(providers) = toml_value.get("providers").and_then(|v| v.as_table()) {
        semantic::check_provider_names(providers, &mut diagnostics);
    }

    // 5. Type check - attempt full deserialization
    if let Err(e) = toml::from_str::<MoltisConfig>(toml_str) {
        let message = format!("type error: {e}");
        if !semantic::should_suppress_deprecated_conflict_type_error(
            &message,
            &conflicting_replacements,
        ) {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                category: "type-error",
                path: String::new(),
                message,
            });
        }
    }

    // 6. Semantic warnings on parsed config (only if it parses)
    if let Ok(config) = toml::from_str::<MoltisConfig>(toml_str) {
        semantic::check_semantic_warnings(&config, &mut diagnostics);
    }

    ValidationResult {
        diagnostics,
        config_path: None,
    }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let n = a_bytes.len();
    let m = b_bytes.len();
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }

    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr = vec![0; m + 1];
    for (i, &ac) in a_bytes.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &bc) in b_bytes.iter().enumerate() {
            let cost = if ac == bc {
                0
            } else {
                1
            };
            let del = prev[j + 1] + 1;
            let ins = curr[j] + 1;
            let sub = prev[j] + cost;
            curr[j + 1] = del.min(ins).min(sub);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

fn suggest<'a>(needle: &str, candidates: &[&'a str], max_distance: usize) -> Option<&'a str> {
    let mut best: Option<(&'a str, usize)> = None;
    for &candidate in candidates {
        let dist = levenshtein(needle, candidate);
        if dist <= max_distance {
            match best {
                Some((_, best_dist)) if dist < best_dist => best = Some((candidate, dist)),
                None => best = Some((candidate, dist)),
                _ => {},
            }
        }
    }
    best.map(|(candidate, _)| candidate)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
#[path = "validate/tests.rs"]
mod tests;
