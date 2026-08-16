//! Lightweight JSON-Schema-ish validator for tool arguments.
//!
//! This is **not** a general-purpose JSON Schema validator. It only checks the
//! subset of schema features that every built-in `AgentTool` actually uses:
//!
//! - `required` array of field names must be present in `args`
//! - `properties.<field>.type` of `string`, `number`/`integer`, `boolean`,
//!   `object`, `array` must match (scalars only at the top level).
//!
//! The goal is narrow: catch the reflex-retry class where a model emits a tool
//! call with `{}` or omits a required field (issue #658). Deeper validation is
//! still each tool's responsibility.
//!
//! A schema that is not an object, has no `required` array, or is simply `{}`
//! is treated as "no required fields" and always passes — this is deliberate
//! so tools with permissive schemas (or test stubs) are not affected.

use serde_json::Value;

use crate::model::ToolCallArgumentDiagnostic;

/// Error returned when tool arguments fail validation.
#[derive(Debug, Clone)]
pub struct ToolArgError {
    pub missing_required: Vec<String>,
    pub type_mismatches: Vec<TypeMismatch>,
    /// The arguments the runner would have dispatched.
    pub received: Value,
}

#[derive(Debug, Clone)]
pub struct TypeMismatch {
    pub field: String,
    pub expected: String,
    pub actual: String,
}

impl ToolArgError {
    /// Format a directive error message targeted at the LLM.
    ///
    /// The message is intentionally terse, names the exact failure, echoes
    /// what the model sent, and explicitly tells the model not to retry with
    /// identical arguments (see issue #658 for the design rationale).
    #[must_use]
    pub fn to_llm_error_message(&self, tool_name: &str) -> String {
        self.to_llm_error_message_with_argument_diagnostic(tool_name, None)
    }

    #[must_use]
    pub fn to_llm_error_message_with_argument_diagnostic(
        &self,
        tool_name: &str,
        argument_diagnostic: Option<&ToolCallArgumentDiagnostic>,
    ) -> String {
        let mut msg = format!("Tool call rejected before execution by `{tool_name}`.\n");

        if !self.missing_required.is_empty() {
            let list = self.missing_required.join("`, `");
            msg.push_str(&format!("Missing required field(s): `{list}`.\n"));
        }
        for tm in &self.type_mismatches {
            msg.push_str(&format!(
                "Field `{}` has wrong type: expected `{}`, got `{}`.\n",
                tm.field, tm.expected, tm.actual,
            ));
        }

        let received_str = serde_json::to_string(&self.received)
            .unwrap_or_else(|_| "<unserializable>".to_string());
        msg.push_str(&format!("You sent: {received_str}\n"));
        if let Some(diagnostic) = argument_diagnostic {
            msg.push_str(&diagnostic.llm_detail());
            msg.push('\n');
        }
        msg.push_str(
            "Do not retry with the same arguments. If you do not know what arguments to use, \
             respond in plain text and ask the user for clarification.",
        );
        msg
    }

    /// Short single-line description for logs and metrics.
    #[must_use]
    pub fn short_summary(&self) -> String {
        self.short_summary_with_argument_diagnostic(None)
    }

    #[must_use]
    pub fn short_summary_with_argument_diagnostic(
        &self,
        argument_diagnostic: Option<&ToolCallArgumentDiagnostic>,
    ) -> String {
        let mut parts = Vec::new();
        if !self.missing_required.is_empty() {
            parts.push(format!("missing={}", self.missing_required.join(",")));
        }
        if !self.type_mismatches.is_empty() {
            let tm: Vec<String> = self
                .type_mismatches
                .iter()
                .map(|t| format!("{}:{}!={}", t.field, t.expected, t.actual))
                .collect();
            parts.push(format!("type_mismatch={}", tm.join(",")));
        }
        if let Some(diagnostic) = argument_diagnostic {
            parts.push(diagnostic.short_summary());
        }
        parts.join(" ")
    }
}

/// Validate `args` against `schema`.
///
/// Returns `Ok(())` when the schema imposes no checkable constraints or all
/// constraints pass. Returns `Err(ToolArgError)` on the narrow failure class
/// this validator targets.
///
/// # Errors
/// Returns [`ToolArgError`] when required fields are missing or top-level
/// types do not match the schema's declared `properties.<field>.type`.
pub fn validate_tool_args(schema: &Value, args: &Value) -> Result<(), ToolArgError> {
    // Only object schemas have required/properties we can check.
    let Some(schema_obj) = schema.as_object() else {
        return Ok(());
    };

    // Empty schema: pass.
    if schema_obj.is_empty() {
        return Ok(());
    }

    // If no required array AND no properties to type-check, pass.
    let required_list: Vec<String> = schema_obj
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let properties = schema_obj.get("properties").and_then(Value::as_object);

    if required_list.is_empty() && properties.is_none() {
        return Ok(());
    }

    // Args must be an object to satisfy any required field.
    let args_obj = match args.as_object() {
        Some(obj) => obj,
        None => {
            // Non-object args with required fields → all missing.
            if required_list.is_empty() {
                return Ok(());
            }
            return Err(ToolArgError {
                missing_required: required_list,
                type_mismatches: Vec::new(),
                received: args.clone(),
            });
        },
    };

    let mut missing_required = Vec::new();
    for field in &required_list {
        match args_obj.get(field) {
            None => missing_required.push(field.clone()),
            Some(Value::Null) => missing_required.push(field.clone()),
            Some(_) => {},
        }
    }

    let mut type_mismatches = Vec::new();
    if let Some(props) = properties {
        for (field, prop_schema) in props {
            let Some(actual_val) = args_obj.get(field) else {
                continue; // Missing-required is handled above; optional missing is fine.
            };
            if actual_val.is_null() {
                continue;
            }
            let Some(expected_type) = prop_schema.as_object().and_then(|o| o.get("type")) else {
                continue; // No declared type → nothing to check.
            };
            let actual_type = value_type_name(actual_val);
            if !type_matches(expected_type, actual_val) {
                type_mismatches.push(TypeMismatch {
                    field: field.clone(),
                    expected: expected_type_name(expected_type),
                    actual: actual_type.to_string(),
                });
            }
        }
    }

    if missing_required.is_empty() && type_mismatches.is_empty() {
        return Ok(());
    }

    Err(ToolArgError {
        missing_required,
        type_mismatches,
        received: args.clone(),
    })
}

fn type_matches(expected: &Value, value: &Value) -> bool {
    match expected {
        Value::String(expected) => type_matches_single(expected, value),
        Value::Array(expected_types) => expected_types
            .iter()
            .filter_map(Value::as_str)
            .any(|expected| type_matches_single(expected, value)),
        _ => true,
    }
}

fn type_matches_single(expected: &str, value: &Value) -> bool {
    match expected {
        "string" => value.is_string(),
        "number" => value.is_number(),
        // Some LLMs serialize integers with a trailing decimal (e.g.
        // `"timeout": 30.0`). Accept integer-valued floats to avoid spurious
        // rejections that would contribute to loop-detector churn rather than
        // catching real reflex loops.
        "integer" => {
            value.as_i64().is_some()
                || value.as_u64().is_some()
                || value.as_f64().is_some_and(|f| f.fract() == 0.0)
        },
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "null" => value.is_null(),
        // Unknown/complex types (unions, $ref, etc.): don't claim a mismatch.
        _ => true,
    }
}

fn expected_type_name(expected: &Value) -> String {
    match expected {
        Value::String(single) => single.clone(),
        Value::Array(types) => {
            let labels: Vec<&str> = types.iter().filter_map(Value::as_str).collect();
            if labels.is_empty() {
                "unknown".to_string()
            } else {
                labels.join(" | ")
            }
        },
        _ => "unknown".to_string(),
    }
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Coerce stringified scalar arguments into the scalar type the schema declares.
///
/// Smaller models frequently emit scalars as JSON strings — e.g.
/// `{"full_page": "true"}` or `{"timeout_ms": "5000"}`. Both this validator and
/// the target tool's serde deserialization would otherwise reject them
/// (`expected boolean, got string` / `invalid type: string`), failing the whole
/// call over a purely cosmetic type slip.
///
/// For each top-level field whose schema declares a `boolean`, `integer`, or
/// `number` type (and does **not** also permit `string`), a string value that
/// parses unambiguously to that scalar is rewritten in place. Anything that does
/// not parse cleanly is left untouched for [`validate_tool_args`] to flag.
///
/// This mirrors the existing leniency in `type_matches_single` that accepts
/// integer-valued floats for `integer`: the goal is to avoid spurious
/// rejections that drive reflex-retry churn, not to mask genuine mistakes.
/// Coercion is deliberately limited to top-level scalars, matching the
/// validator's own scope. Call it immediately before [`validate_tool_args`] so
/// the coerced object is both validated and dispatched.
pub fn coerce_scalar_args(schema: &Value, args: &mut Value) {
    let Some(props) = schema
        .as_object()
        .and_then(|obj| obj.get("properties"))
        .and_then(Value::as_object)
    else {
        return;
    };
    let Some(args_obj) = args.as_object_mut() else {
        return;
    };

    // Snapshot which fields to rewrite first: `props` borrows `schema`, but the
    // rewrite needs a mutable borrow of `args_obj`, so the two cannot overlap.
    let mut rewrites: Vec<(String, Value)> = Vec::new();
    for (field, prop_schema) in props {
        let Some(Value::String(raw)) = args_obj.get(field) else {
            continue; // only string inputs are candidates for coercion
        };
        let Some(expected) = prop_schema.as_object().and_then(|obj| obj.get("type")) else {
            continue; // no declared type → nothing to coerce toward
        };
        let allowed = type_labels(expected);
        // If a string is itself valid for this field, the model may genuinely
        // have meant a string — leave it untouched.
        if allowed.contains(&"string") {
            continue;
        }
        if let Some(coerced) = coerce_string_scalar(raw, &allowed) {
            rewrites.push((field.clone(), coerced));
        }
    }

    for (field, coerced) in rewrites {
        args_obj.insert(field, coerced);
    }
}

/// Collect the declared JSON Schema `type` labels, handling both the scalar
/// (`"boolean"`) and union (`["integer", "null"]`) forms.
fn type_labels(expected: &Value) -> Vec<&str> {
    match expected {
        Value::String(single) => vec![single.as_str()],
        Value::Array(types) => types.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

/// Parse `raw` into the first scalar type in `allowed` it unambiguously matches.
///
/// Returns `None` when nothing parses cleanly, leaving the original string for
/// the validator to reject.
fn coerce_string_scalar(raw: &str, allowed: &[&str]) -> Option<Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    for label in allowed {
        match *label {
            "boolean" => {
                if trimmed.eq_ignore_ascii_case("true") {
                    return Some(Value::Bool(true));
                }
                if trimmed.eq_ignore_ascii_case("false") {
                    return Some(Value::Bool(false));
                }
            },
            "integer" => {
                if let Ok(signed) = trimmed.parse::<i64>() {
                    return Some(Value::Number(signed.into()));
                }
                if let Ok(unsigned) = trimmed.parse::<u64>() {
                    return Some(Value::Number(unsigned.into()));
                }
            },
            "number" => {
                if let Ok(float) = trimmed.parse::<f64>()
                    && let Some(number) = serde_json::Number::from_f64(float)
                {
                    return Some(Value::Number(number));
                }
            },
            _ => {},
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use {
        super::*,
        crate::model::{ToolCallArgumentDiagnostic, ToolCallArgumentSource},
        serde_json::json,
    };

    #[test]
    fn empty_schema_always_passes() {
        assert!(validate_tool_args(&json!({}), &json!({})).is_ok());
        assert!(validate_tool_args(&json!({}), &json!({"x": 1})).is_ok());
    }

    #[test]
    fn schema_without_required_passes_on_empty_args() {
        let schema = json!({
            "type": "object",
            "properties": { "command": { "type": "string" } }
        });
        assert!(validate_tool_args(&schema, &json!({})).is_ok());
    }

    #[test]
    fn missing_required_field_is_reported() {
        let schema = json!({
            "type": "object",
            "properties": { "command": { "type": "string" } },
            "required": ["command"]
        });
        let err = validate_tool_args(&schema, &json!({})).unwrap_err();
        assert_eq!(err.missing_required, vec!["command".to_string()]);
        assert!(err.type_mismatches.is_empty());
    }

    #[test]
    fn validation_error_can_include_argument_decode_diagnostic() {
        let schema = json!({
            "type": "object",
            "properties": { "command": { "type": "string" } },
            "required": ["command"]
        });
        let err = validate_tool_args(&schema, &json!({})).unwrap_err();
        let diagnostic = ToolCallArgumentDiagnostic {
            source: ToolCallArgumentSource::EmptyString,
            raw_len: Some(0),
            raw_preview: Some(String::new()),
            parse_error: None,
        };

        assert_eq!(
            err.short_summary_with_argument_diagnostic(Some(&diagnostic)),
            "missing=command arg_decode=empty-string raw_len=0"
        );
        let message = err.to_llm_error_message_with_argument_diagnostic("exec", Some(&diagnostic));
        assert!(message.contains("Argument decode status: arg_decode=empty-string raw_len=0."));
        assert!(!message.contains("Raw argument preview:"));
    }

    #[test]
    fn null_field_counts_as_missing() {
        let schema = json!({
            "type": "object",
            "properties": { "command": { "type": "string" } },
            "required": ["command"]
        });
        let err = validate_tool_args(&schema, &json!({"command": null})).unwrap_err();
        assert_eq!(err.missing_required, vec!["command".to_string()]);
    }

    #[test]
    fn wrong_type_is_reported() {
        let schema = json!({
            "type": "object",
            "properties": { "command": { "type": "string" } },
            "required": ["command"]
        });
        let err = validate_tool_args(&schema, &json!({"command": 42})).unwrap_err();
        assert!(err.missing_required.is_empty());
        assert_eq!(err.type_mismatches.len(), 1);
        assert_eq!(err.type_mismatches[0].field, "command");
        assert_eq!(err.type_mismatches[0].expected, "string");
        assert_eq!(err.type_mismatches[0].actual, "number");
    }

    #[test]
    fn multiple_required_missing() {
        let schema = json!({
            "type": "object",
            "properties": {
                "a": { "type": "string" },
                "b": { "type": "string" }
            },
            "required": ["a", "b"]
        });
        let err = validate_tool_args(&schema, &json!({})).unwrap_err();
        assert_eq!(err.missing_required.len(), 2);
    }

    #[test]
    fn valid_args_pass() {
        let schema = json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "cwd": { "type": "string" }
            },
            "required": ["command"]
        });
        assert!(validate_tool_args(&schema, &json!({"command": "ls", "cwd": "/tmp"})).is_ok());
    }

    #[test]
    fn optional_field_wrong_type_still_reports() {
        let schema = json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "timeout": { "type": "integer" }
            },
            "required": ["command"]
        });
        let err =
            validate_tool_args(&schema, &json!({"command": "ls", "timeout": "slow"})).unwrap_err();
        assert!(err.missing_required.is_empty());
        assert_eq!(err.type_mismatches.len(), 1);
        assert_eq!(err.type_mismatches[0].field, "timeout");
    }

    #[test]
    fn non_object_args_with_required_fails() {
        let schema = json!({
            "type": "object",
            "properties": { "command": { "type": "string" } },
            "required": ["command"]
        });
        let err = validate_tool_args(&schema, &json!("ls")).unwrap_err();
        assert_eq!(err.missing_required, vec!["command".to_string()]);
    }

    #[test]
    fn unknown_type_is_permissive() {
        let schema = json!({
            "type": "object",
            "properties": { "x": { "type": "some_future_thing" } },
            "required": ["x"]
        });
        assert!(validate_tool_args(&schema, &json!({"x": "anything"})).is_ok());
    }

    #[test]
    fn array_and_object_types() {
        let schema = json!({
            "type": "object",
            "properties": {
                "items": { "type": "array" },
                "meta":  { "type": "object" }
            },
            "required": ["items", "meta"]
        });
        assert!(validate_tool_args(&schema, &json!({"items": [1,2], "meta": {"k": "v"}})).is_ok());
        let err =
            validate_tool_args(&schema, &json!({"items": "not-an-array", "meta": {}})).unwrap_err();
        assert_eq!(err.type_mismatches.len(), 1);
        assert_eq!(err.type_mismatches[0].field, "items");
    }

    #[test]
    fn llm_error_message_is_directive() {
        let schema = json!({
            "type": "object",
            "properties": { "command": { "type": "string" } },
            "required": ["command"]
        });
        let err = validate_tool_args(&schema, &json!({})).unwrap_err();
        let msg = err.to_llm_error_message("exec");
        assert!(msg.contains("exec"));
        assert!(msg.contains("command"));
        assert!(msg.contains("Do not retry"));
        assert!(msg.contains("respond in plain text"));
    }

    #[test]
    fn integer_accepts_integer_valued_floats() {
        // Some LLMs (e.g. via OpenAI JSON-mode) emit integers with a trailing
        // decimal point. Schema says "integer" — we must not reject 30.0.
        let schema = json!({
            "type": "object",
            "properties": { "timeout": { "type": "integer" } },
            "required": ["timeout"]
        });
        assert!(validate_tool_args(&schema, &json!({"timeout": 30})).is_ok());
        assert!(validate_tool_args(&schema, &json!({"timeout": 30.0})).is_ok());
        // A non-integer float must still be rejected.
        let err = validate_tool_args(&schema, &json!({"timeout": 30.5})).unwrap_err();
        assert_eq!(err.type_mismatches.len(), 1);
        assert_eq!(err.type_mismatches[0].field, "timeout");
    }

    #[test]
    fn union_type_array_accepts_any_matching_type() {
        let schema = json!({
            "type": "object",
            "properties": {
                "schedule": { "type": ["object", "string", "integer"] },
                "maybe_null": { "type": ["string", "null"] }
            },
            "required": ["schedule"]
        });

        assert!(
            validate_tool_args(
                &schema,
                &json!({"schedule": {"kind": "at"}, "maybe_null": "ok"})
            )
            .is_ok()
        );
        assert!(
            validate_tool_args(
                &schema,
                &json!({"schedule": "0 9 * * *", "maybe_null": null})
            )
            .is_ok()
        );
        assert!(
            validate_tool_args(
                &schema,
                &json!({"schedule": 1_700_000_000_000u64, "maybe_null": "ok"})
            )
            .is_ok()
        );
    }

    #[test]
    fn union_type_array_reports_mismatch_when_nothing_matches() {
        let schema = json!({
            "type": "object",
            "properties": {
                "schedule": { "type": ["object", "string", "integer"] }
            },
            "required": ["schedule"]
        });

        let err = validate_tool_args(&schema, &json!({"schedule": false})).unwrap_err();
        assert_eq!(err.type_mismatches.len(), 1);
        assert_eq!(err.type_mismatches[0].expected, "object | string | integer");
        assert_eq!(err.type_mismatches[0].actual, "boolean");
    }

    #[test]
    fn short_summary_captures_both_kinds() {
        let schema = json!({
            "type": "object",
            "properties": {
                "a": { "type": "string" },
                "b": { "type": "integer" }
            },
            "required": ["a", "b"]
        });
        let err = validate_tool_args(&schema, &json!({"b": "wrong"})).unwrap_err();
        let s = err.short_summary();
        assert!(s.contains("missing=a"));
        assert!(s.contains("type_mismatch=b:integer!=string"));
    }

    /// The browser-tool failure class that motivated this change: a small model
    /// sends `full_page` as the string `"true"` and an integer field as a
    /// numeric string. After coercion the call validates cleanly.
    #[test]
    fn coerces_browser_style_stringified_scalars() {
        let schema = json!({
            "type": "object",
            "properties": {
                "full_page": { "type": "boolean" },
                "timeout_ms": { "type": "integer" },
                "url": { "type": "string" }
            }
        });
        let mut args = json!({
            "full_page": "true",
            "timeout_ms": "5000",
            "url": "https://example.com"
        });
        // Pre-coercion these would be rejected by the validator.
        assert!(validate_tool_args(&schema, &args).is_err());

        coerce_scalar_args(&schema, &mut args);

        assert_eq!(args["full_page"], json!(true));
        assert_eq!(args["timeout_ms"], json!(5000));
        // Genuine strings are untouched.
        assert_eq!(args["url"], json!("https://example.com"));
        assert!(validate_tool_args(&schema, &args).is_ok());
    }

    #[test]
    fn coerces_false_and_negative_and_float() {
        let schema = json!({
            "type": "object",
            "properties": {
                "flag": { "type": "boolean" },
                "offset": { "type": "integer" },
                "ratio": { "type": "number" }
            }
        });
        let mut args = json!({"flag": "False", "offset": "-12", "ratio": "0.5"});
        coerce_scalar_args(&schema, &mut args);
        assert_eq!(args["flag"], json!(false));
        assert_eq!(args["offset"], json!(-12));
        assert_eq!(args["ratio"], json!(0.5));
    }

    /// When a field's schema also permits `string`, the model may have meant a
    /// string — never coerce it.
    #[test]
    fn leaves_string_union_fields_alone() {
        let schema = json!({
            "type": "object",
            "properties": {
                "schedule": { "type": ["integer", "string"] }
            }
        });
        let mut args = json!({"schedule": "5"});
        coerce_scalar_args(&schema, &mut args);
        assert_eq!(
            args["schedule"],
            json!("5"),
            "string-typed union must stay a string"
        );
    }

    /// Unparseable strings are left for the validator to reject, not mangled.
    #[test]
    fn leaves_unparseable_strings_for_the_validator() {
        let schema = json!({
            "type": "object",
            "properties": {
                "count": { "type": "integer" },
                "flag": { "type": "boolean" }
            }
        });
        let mut args = json!({"count": "not-a-number", "flag": "maybe"});
        coerce_scalar_args(&schema, &mut args);
        assert_eq!(args["count"], json!("not-a-number"));
        assert_eq!(args["flag"], json!("maybe"));
        assert!(validate_tool_args(&schema, &args).is_err());
    }

    /// A nullable union (`["integer", "null"]`) with no `string` still coerces.
    #[test]
    fn coerces_into_nullable_numeric_union() {
        let schema = json!({
            "type": "object",
            "properties": { "ref_": { "type": ["integer", "null"] } }
        });
        let mut args = json!({"ref_": "14"});
        coerce_scalar_args(&schema, &mut args);
        assert_eq!(args["ref_"], json!(14));
    }

    /// Non-object args and schemas without `properties` are no-ops.
    #[test]
    fn coercion_is_a_noop_for_unstructured_inputs() {
        let mut scalar = json!("plain");
        coerce_scalar_args(
            &json!({"properties": {"x": {"type": "integer"}}}),
            &mut scalar,
        );
        assert_eq!(scalar, json!("plain"));

        let mut args = json!({"x": "5"});
        coerce_scalar_args(&json!({}), &mut args);
        assert_eq!(
            args["x"],
            json!("5"),
            "no properties → nothing to coerce toward"
        );
    }
}
