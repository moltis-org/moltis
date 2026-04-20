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
    tracing::debug,
};

/// Whether a property schema contains at least one type-defining keyword.
///
/// Google AI Studio (as opposed to Vertex AI) rejects properties that have
/// only a `description` but no structural type info. This helper is used
/// by [`PruneOrphanedRequiredTransform`] and [`EnsurePropertyTypeTransform`]
/// to decide which properties are "defined" (#793).
fn has_usable_type(v: &serde_json::Value) -> bool {
    let Some(obj) = v.as_object() else {
        // Boolean schema: `true` / `false` — not a concrete type definition.
        return false;
    };
    if obj.is_empty() {
        return false;
    }
    const TYPE_KEYWORDS: &[&str] = &["type", "enum", "anyOf", "oneOf", "allOf", "$ref"];
    TYPE_KEYWORDS.iter().any(|k| obj.contains_key(*k))
}

/// Infer `"type": "string"` for required properties that have a `description`
/// but no type-defining keyword.
///
/// Google AI Studio rejects schemas where `required` references a property
/// that only has a `description`. Rather than pruning the property name
/// (which loses context for the LLM), we infer the safest type (#793).
#[derive(Debug, Clone, Default)]
struct EnsurePropertyTypeTransform;

impl Transform for EnsurePropertyTypeTransform {
    fn transform(&mut self, schema: &mut Schema) {
        let Some(obj) = schema.as_object_mut() else {
            return;
        };

        let required_names: BTreeSet<String> = obj
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| e.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if required_names.is_empty() {
            return;
        }

        let Some(props) = obj.get_mut("properties").and_then(|v| v.as_object_mut()) else {
            return;
        };

        for name in &required_names {
            if let Some(prop_schema) = props.get_mut(name) {
                // Skip bare boolean `true` (canonicalized "accept anything"
                // schema) and empty `{}` — these are genuinely undefined
                // properties and should be handled by pruning, not promoted.
                if prop_schema.as_bool().is_some()
                    || prop_schema.as_object().is_some_and(|o| o.is_empty())
                {
                    continue;
                }
                // Object with metadata (e.g. `description`) but no
                // type-defining keyword — infer `"type": "string"` so the
                // property name is preserved for the LLM (#793).
                let needs_type = prop_schema
                    .as_object()
                    .is_some_and(|o| !o.is_empty() && !has_usable_type(prop_schema));
                if needs_type && let Some(prop_obj) = prop_schema.as_object_mut() {
                    prop_obj.insert(
                        "type".to_string(),
                        serde_json::Value::String("string".to_string()),
                    );
                }
            }
        }
    }
}

/// Prune `required` entries that reference properties not defined in `properties`.
///
/// MCP tools (e.g. Home Assistant with 80+ tools) may declare `required`
/// entries for properties defined via unsupported keywords (`dependentSchemas`,
/// `if`/`then`/`else`, `patternProperties`) that get stripped by
/// `OpenAiSchemaSubsetTransform`. Gemini strictly validates that every
/// `required` entry has a matching property and rejects the request with
/// "property is not defined" when they don't match (issue #747).
#[derive(Debug, Clone, Default)]
struct PruneOrphanedRequiredTransform;

impl Transform for PruneOrphanedRequiredTransform {
    fn transform(&mut self, schema: &mut Schema) {
        let Some(obj) = schema.as_object_mut() else {
            return;
        };

        // Collect property names that have a meaningful, usable schema.
        // A property must have at least one type-defining keyword to be
        // considered defined.  Properties with bare boolean schemas (`true`),
        // empty objects (`{}`), or only `description` (no type info) are
        // treated as undefined because Google AI Studio rejects them with
        // "property is not defined" (issues #747, #793).
        let defined_props: BTreeSet<String> = obj
            .get("properties")
            .and_then(|v| v.as_object())
            .map(|props| {
                props
                    .iter()
                    .filter(|(_, v)| has_usable_type(v))
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default();

        if defined_props.is_empty() {
            // No usable properties — `required` is entirely orphaned.
            obj.remove("required");
            return;
        }

        if let Some(required) = obj.get_mut("required").and_then(|v| v.as_array_mut()) {
            required.retain(|entry| {
                entry
                    .as_str()
                    .is_some_and(|name| defined_props.contains(name))
            });
        }
        // Remove `required` entirely when retain emptied it.
        if obj
            .get("required")
            .and_then(|v| v.as_array())
            .is_some_and(|a| a.is_empty())
        {
            obj.remove("required");
        }
    }
}

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

/// Recursively strip `$schema` from all levels of a JSON value.
///
/// `json_schema_ast` validates `$schema` at every nesting level (properties,
/// definitions, `$defs`, etc.) and rejects non-Draft-2020-12 URIs. MCP tools
/// may embed `$schema` in nested definitions (issue #760), so root-only
/// stripping is insufficient.
fn strip_schema_keyword_recursive(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(obj) => {
            obj.remove("$schema");
            for v in obj.values_mut() {
                strip_schema_keyword_recursive(v);
            }
        },
        serde_json::Value::Array(arr) => {
            for item in arr {
                strip_schema_keyword_recursive(item);
            }
        },
        _ => {},
    }
}

fn canonicalize_schema_for_openai_compat(schema: &serde_json::Value) -> serde_json::Value {
    // Recursively strip `$schema` so `SchemaDocument::from_json()` doesn't
    // reject non-2020-12 drafts (e.g. draft-07 from Attio MCP tools,
    // issue #743, #760). Draft-07 and 2020-12 share enough structural
    // keywords that canonicalization works; remaining differences
    // (`definitions` vs `$defs`, tuple `items` vs `prefixItems`) are
    // handled by schemars transforms downstream. `$schema` itself is later
    // stripped by `OpenAiSchemaSubsetTransform` anyway.
    let mut input = schema.clone();
    strip_schema_keyword_recursive(&mut input);

    let document = match SchemaDocument::from_json(&input) {
        Ok(document) => document,
        Err(error) => {
            debug!(
                error = %error,
                "openai tool schema failed Draft 2020-12 preflight; using raw schema for best-effort normalization"
            );
            return input;
        },
    };

    if let Err(error) = document.root() {
        debug!(
            error = %error,
            "openai tool schema failed canonical AST resolution; using raw schema for best-effort normalization"
        );
        return input;
    }

    document
        .canonical_schema_json()
        .map_or_else(
            |error| {
                debug!(
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

    // Infer `"type": "string"` for required properties that have metadata
    // (e.g. `description`) but no type-defining keyword. Must run before
    // pruning so the property is considered "defined" (#793).
    let mut ensure_type = RecursiveTransform(EnsurePropertyTypeTransform);
    ensure_type.transform(&mut transformed);

    // Prune `required` entries that reference properties not defined in
    // `properties`. Keyword stripping above can remove property definitions
    // (e.g. `dependentSchemas`, `if/then/else`) while leaving their names
    // in `required`. Gemini/Google AI Studio rejects such schemas (#747, #793).
    let mut prune_orphaned_required = RecursiveTransform(PruneOrphanedRequiredTransform);
    prune_orphaned_required.transform(&mut transformed);

    // Re-infer `"type"` from enum values after canonicalization stripped it.
    // Providers like Fireworks AI reject schemas without explicit type
    // annotations even when enum values unambiguously imply the type.
    let mut restore_enum_type = RecursiveTransform(RestoreEnumTypeTransform);
    restore_enum_type.transform(&mut transformed);

    *schema = transformed.to_value();
}
