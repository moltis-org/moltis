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
        msg.push_str(
            "Do not retry with the same arguments. If you do not know what arguments to use, \
             respond in plain text and ask the user for clarification.",
        );
        msg
    }

    /// Short single-line description for logs and metrics.
    #[must_use]
    pub fn short_summary(&self) -> String {
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use {super::*, serde_json::json};

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
}
