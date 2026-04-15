use std::collections::HashSet;

/// Make a JSON Schema node nullable by adding `"null"` to its type.
///
/// For schemas using `"type": "string"` (or any single type), this converts to
/// `"type": ["string", "null"]`. For schemas using `anyOf`/`oneOf`, it appends
/// a `{"type": "null"}` variant. Already-nullable schemas are left unchanged.
fn make_nullable(schema: &mut serde_json::Value) {
    let Some(obj) = schema.as_object_mut() else {
        return;
    };

    if let Some(ty) = obj.get("type") {
        if ty.as_str() == Some("null") {
            return;
        }
        if let Some(arr) = ty.as_array()
            && arr.iter().any(|value| value.as_str() == Some("null"))
        {
            return;
        }
    }

    if let Some(ty) = obj.get("type").cloned() {
        if let Some(kind) = ty.as_str() {
            obj.insert("type".to_string(), serde_json::json!([kind, "null"]));
        } else if let Some(arr) = ty.as_array() {
            let mut new_arr = arr.clone();
            new_arr.push(serde_json::json!("null"));
            obj.insert("type".to_string(), serde_json::Value::Array(new_arr));
        }
        return;
    }

    for key in ["anyOf", "oneOf"] {
        if let Some(variants) = obj.get_mut(key).and_then(|value| value.as_array_mut()) {
            let has_null = variants
                .iter()
                .any(|variant| variant.get("type").and_then(|kind| kind.as_str()) == Some("null"));
            if !has_null {
                variants.push(serde_json::json!({ "type": "null" }));
            }
            return;
        }
    }
}

/// Check whether a schema's `type` field designates an object.
///
/// Matches both `"type": "object"` and `"type": ["object", ...]`.
/// Downstream providers like Gemini cannot represent type arrays, and OpenAI
/// strict mode requires `type` to be a single string.
fn type_is_object(obj: &serde_json::Map<String, serde_json::Value>) -> bool {
    let Some(ty) = obj.get("type") else {
        return false;
    };

    match ty {
        serde_json::Value::String(kind) => kind == "object",
        serde_json::Value::Array(kinds) => kinds.iter().any(|kind| kind.as_str() == Some("object")),
        _ => false,
    }
}

/// Recursively patch schema for OpenAI strict mode compliance.
///
/// OpenAI's strict mode requires:
/// 1. `additionalProperties: false` on every object in the schema tree
/// 2. All properties must be listed in the `required` array
///
/// Properties not in the original `required` array are made nullable so the
/// model can send `null` for unused optional fields instead of fabricating
/// placeholder values.
///
/// Schemas with `type: ["object", "string", ...]` are collapsed to
/// `type: "object"` because strict mode and downstream OpenAI-compatible
/// providers require a single concrete object type for tool schemas.
pub fn patch_schema_for_strict_mode(schema: &mut serde_json::Value) {
    let Some(obj) = schema.as_object_mut() else {
        return;
    };

    let mut optional_props: Vec<String> = Vec::new();

    if type_is_object(obj) {
        if obj.get("type").and_then(|value| value.as_array()).is_some() {
            obj.insert("type".to_string(), serde_json::json!("object"));
        }

        obj.insert("additionalProperties".to_string(), serde_json::json!(false));

        if let Some(props) = obj
            .get("properties")
            .and_then(|value| value.as_object())
            .cloned()
        {
            let originally_required: HashSet<String> = obj
                .get("required")
                .and_then(|value| value.as_array())
                .map(|required| {
                    required
                        .iter()
                        .filter_map(|entry| entry.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            let all_prop_names: Vec<serde_json::Value> =
                props.keys().map(|name| serde_json::json!(name)).collect();
            obj.insert("required".to_string(), serde_json::json!(all_prop_names));

            for key in props.keys() {
                if !originally_required.contains(key) {
                    optional_props.push(key.clone());
                }
            }
        } else {
            obj.insert("properties".to_string(), serde_json::json!({}));
            obj.insert("required".to_string(), serde_json::json!([]));
        }
    }

    if let Some(props) = obj
        .get_mut("properties")
        .and_then(|value| value.as_object_mut())
    {
        for prop_schema in props.values_mut() {
            patch_schema_for_strict_mode(prop_schema);
        }
    }

    if let Some(items) = obj.get_mut("items") {
        patch_schema_for_strict_mode(items);
    }

    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(variants) = obj.get_mut(key).and_then(|value| value.as_array_mut()) {
            for variant in variants {
                patch_schema_for_strict_mode(variant);
            }
        }
    }

    if let Some(additional) = obj.get_mut("additionalProperties")
        && additional.is_object()
    {
        patch_schema_for_strict_mode(additional);
    }

    if !optional_props.is_empty()
        && let Some(props) = obj
            .get_mut("properties")
            .and_then(|value| value.as_object_mut())
    {
        for key in &optional_props {
            if let Some(prop_schema) = props.get_mut(key) {
                make_nullable(prop_schema);
            }
        }
    }
}
