use {
    json_schema_ast::SchemaDocument,
    schemars::{
        Schema,
        transform::{
            RecursiveTransform, RemoveRefSiblings, ReplaceConstValue, ReplacePrefixItems,
            ReplaceUnevaluatedProperties, Transform,
        },
    },
    std::collections::BTreeSet,
    tracing::warn,
};

/// Re-infer `"type"` from `"enum"` values when canonicalization stripped it.
///
/// `json_schema_ast` canonicalization removes redundant `"type"` annotations
/// when all enum values match the declared type (`lower_enum_with_type`), and
/// converts `"type": "boolean"` → `"enum": [false, true]`
/// (`lower_boolean_and_null_types`). This is correct per JSON Schema semantics
/// but providers like Fireworks AI reject schemas without explicit `"type"`.
///
/// This transform walks every schema node and restores `"type"` when:
/// - `"enum"` is present but `"type"` is absent
/// - All non-null enum values share a single JSON type
#[derive(Debug, Clone, Default)]
struct RestoreEnumTypeTransform;

impl Transform for RestoreEnumTypeTransform {
    fn transform(&mut self, schema: &mut Schema) {
        let Some(obj) = schema.as_object_mut() else {
            return;
        };

        // Only act when `enum` is present and `type` is absent.
        if obj.contains_key("type") {
            return;
        }
        let Some(values) = obj.get("enum").and_then(|v| v.as_array()) else {
            return;
        };
        if values.is_empty() {
            return;
        }

        // Collect the distinct JSON types of non-null enum values.
        let mut types = BTreeSet::new();
        for value in values {
            match value {
                serde_json::Value::Null => {}, // ignore null for type inference
                serde_json::Value::Bool(_) => {
                    types.insert("boolean");
                },
                serde_json::Value::Number(n) => {
                    if n.is_f64() && !n.is_i64() && !n.is_u64() {
                        types.insert("number");
                    } else {
                        types.insert("integer");
                    }
                },
                serde_json::Value::String(_) => {
                    types.insert("string");
                },
                serde_json::Value::Array(_) => {
                    types.insert("array");
                },
                serde_json::Value::Object(_) => {
                    types.insert("object");
                },
            }
        }

        // In JSON Schema, "number" subsumes "integer". When both appear
        // (e.g. enum mixes 1 and 1.5), collapse to "number".
        if types.contains("integer") && types.contains("number") {
            types.remove("integer");
        }

        // Only restore when all non-null values share a single type.
        if types.len() == 1 {
            let inferred_type = types.into_iter().next().unwrap_or_default();
            obj.insert(
                "type".to_string(),
                serde_json::Value::String(inferred_type.to_string()),
            );
        }
    }
}

/// Remove empty `{}` (the JSON Schema "true" schema) from `anyOf`/`oneOf`
/// composite arrays and collapse single-variant composites inline.
///
/// Canonicalization of `not` and other negation keywords produces `{}` (the
/// "accepts anything" schema). After keyword stripping, these survive as
/// empty objects inside `anyOf`/`oneOf`, which OpenAI rejects with
/// "schema must have a 'type' key" (issue #743).
#[derive(Debug, Clone, Default)]
struct SimplifyCompositeTransform;

impl Transform for SimplifyCompositeTransform {
    fn transform(&mut self, schema: &mut Schema) {
        let Some(obj) = schema.as_object_mut() else {
            return;
        };

        for keyword in ["anyOf", "oneOf", "allOf"] {
            let Some(variants) = obj.get_mut(keyword).and_then(|v| v.as_array_mut()) else {
                continue;
            };

            // Drop empty-object variants (`{}`).
            variants.retain(|v| !v.as_object().is_some_and(|o| o.is_empty()));

            if variants.len() == 1 {
                // Single variant left — inline it, replacing the composite.
                let single = variants.remove(0);
                obj.remove(keyword);
                if let serde_json::Value::Object(inner) = single {
                    for (k, v) in inner {
                        // Parent-key wins: if a key is already present (e.g. `type`
                        // from a surrounding object schema), we keep the parent value
                        // and discard the variant's. This is safe for the `not`→`{}`
                        // canonicalization pattern this transform targets.
                        obj.entry(k).or_insert(v);
                    }
                }
            } else if variants.is_empty() {
                obj.remove(keyword);
            }
        }
    }
}

const OPENAI_ALLOWED_SCHEMA_KEYWORDS: &[&str] = &[
    "$ref",
    "$defs",
    "definitions",
    "type",
    "enum",
    "title",
    "description",
    "default",
    "example",
    "examples",
    "format",
    "pattern",
    "properties",
    "required",
    "items",
    "additionalProperties",
    "anyOf",
    "oneOf",
    "allOf",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
    "uniqueItems",
];

#[derive(Debug, Clone, Default)]
struct OpenAiSchemaSubsetTransform;

impl Transform for OpenAiSchemaSubsetTransform {
    fn transform(&mut self, schema: &mut Schema) {
        let Some(obj) = schema.as_object_mut() else {
            return;
        };

        obj.retain(|key, _| OPENAI_ALLOWED_SCHEMA_KEYWORDS.contains(&key.as_str()));
    }
}

fn canonicalize_schema_for_openai_compat(schema: &serde_json::Value) -> serde_json::Value {
    // Strip `$schema` so `SchemaDocument::from_json()` doesn't reject
    // non-2020-12 drafts (e.g. draft-07 from Attio MCP tools, issue #743).
    // Draft-07 and 2020-12 share enough structural keywords that
    // canonicalization works; remaining differences (`definitions` vs
    // `$defs`, tuple `items` vs `prefixItems`) are handled by schemars
    // transforms downstream. `$schema` itself is later stripped by
    // `OpenAiSchemaSubsetTransform` anyway.
    let mut input = schema.clone();
    if let Some(obj) = input.as_object_mut() {
        obj.remove("$schema");
    }

    let document = match SchemaDocument::from_json(&input) {
        Ok(document) => document,
        Err(error) => {
            warn!(
                error = %error,
                "openai tool schema failed Draft 2020-12 preflight; using raw schema for best-effort normalization"
            );
            return input;
        },
    };

    if let Err(error) = document.root() {
        warn!(
            error = %error,
            "openai tool schema failed canonical AST resolution; using raw schema for best-effort normalization"
        );
        return input;
    }

    document
        .canonical_schema_json()
        .map_or_else(
            |error| {
                warn!(
                    error = %error,
                    "openai tool schema canonicalization was unavailable; using raw schema for best-effort normalization"
                );
                input
            },
            serde_json::Value::clone,
        )
}

/// Validate and normalize a JSON Schema document into the OpenAI-compatible
/// function-calling subset via `json_schema_ast` canonicalization plus
/// recursive `schemars` transforms.
pub(crate) fn sanitize_schema_for_openai_compat(schema: &mut serde_json::Value) {
    let canonical = canonicalize_schema_for_openai_compat(schema);

    let Ok(mut transformed) = Schema::try_from(canonical.clone()) else {
        *schema = canonical;
        return;
    };
    let mut replace_const = ReplaceConstValue::default();
    replace_const.transform(&mut transformed);
    let mut replace_unevaluated_properties = ReplaceUnevaluatedProperties::default();
    replace_unevaluated_properties.transform(&mut transformed);
    let mut replace_prefix_items = ReplacePrefixItems::default();
    replace_prefix_items.transform(&mut transformed);
    let mut remove_ref_siblings = RemoveRefSiblings::default();
    remove_ref_siblings.transform(&mut transformed);
    let mut subset_transform = RecursiveTransform(OpenAiSchemaSubsetTransform);
    subset_transform.transform(&mut transformed);

    // Strip empty `{}` schemas from anyOf/oneOf (left behind by
    // canonicalization of `not` and other negation keywords) and collapse
    // single-variant composites inline (issue #743).
    let mut simplify_composite = RecursiveTransform(SimplifyCompositeTransform);
    simplify_composite.transform(&mut transformed);

    // Re-infer `"type"` from enum values after canonicalization stripped it.
    // Providers like Fireworks AI reject schemas without explicit type
    // annotations even when enum values unambiguously imply the type.
    let mut restore_enum_type = RecursiveTransform(RestoreEnumTypeTransform);
    restore_enum_type.transform(&mut transformed);

    *schema = transformed.to_value();
}
