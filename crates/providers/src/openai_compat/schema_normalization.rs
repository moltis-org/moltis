use {
    json_schema_ast::SchemaDocument,
    schemars::{
        Schema,
        transform::{
            RecursiveTransform, RemoveRefSiblings, ReplaceConstValue, ReplacePrefixItems,
            ReplaceUnevaluatedProperties, Transform,
        },
    },
    tracing::warn,
};

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
    let document = match SchemaDocument::from_json(schema) {
        Ok(document) => document,
        Err(error) => {
            warn!(
                error = %error,
                "openai tool schema failed Draft 2020-12 preflight; using raw schema for best-effort normalization"
            );
            return schema.clone();
        },
    };

    if let Err(error) = document.root() {
        warn!(
            error = %error,
            "openai tool schema failed canonical AST resolution; using raw schema for best-effort normalization"
        );
        return schema.clone();
    }

    document
        .canonical_schema_json()
        .map_or_else(
            |error| {
                warn!(
                    error = %error,
                    "openai tool schema canonicalization was unavailable; using raw schema for best-effort normalization"
                );
                schema.clone()
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

    *schema = transformed.to_value();
}
