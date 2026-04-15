use moltis_providers::openai_compat::{patch_schema_for_strict_mode, to_openai_tools};

fn array_value<'a>(value: &'a serde_json::Value, context: &str) -> &'a [serde_json::Value] {
    match value.as_array() {
        Some(values) => values,
        None => panic!("{context} should be an array, got {value:?}"),
    }
}

fn object_value<'a>(
    value: &'a serde_json::Value,
    context: &str,
) -> &'a serde_json::Map<String, serde_json::Value> {
    match value.as_object() {
        Some(object) => object,
        None => panic!("{context} should be an object, got {value:?}"),
    }
}

fn str_value<'a>(value: &'a serde_json::Value, context: &str) -> &'a str {
    match value.as_str() {
        Some(string) => string,
        None => panic!("{context} should be a string, got {value:?}"),
    }
}
#[test]
fn strict_mode_collapses_object_union_types() {
    let mut schema = serde_json::json!({
        "type": ["object", "string"],
        "properties": {
            "kind": { "type": "string" },
            "value": { "type": "integer" }
        },
        "required": ["kind"]
    });

    patch_schema_for_strict_mode(&mut schema);

    assert_eq!(schema["type"], "object");
    assert_eq!(schema["additionalProperties"], false);
    let required = array_value(&schema["required"], "required");
    assert_eq!(required.len(), 2);
}

#[test]
fn strict_mode_collapses_triple_union_types() {
    let mut schema = serde_json::json!({
        "type": ["object", "string", "integer"],
        "properties": {
            "kind": { "type": "string" },
            "every_ms": { "type": "integer" }
        },
        "required": ["kind"]
    });

    patch_schema_for_strict_mode(&mut schema);

    assert_eq!(schema["type"], "object");
    assert_eq!(schema["additionalProperties"], false);
}

#[test]
fn strict_mode_leaves_singular_string_type_unchanged() {
    let mut schema = serde_json::json!({
        "type": "string"
    });

    patch_schema_for_strict_mode(&mut schema);

    assert_eq!(schema["type"], "string");
    assert!(schema.get("additionalProperties").is_none());
}

#[test]
fn strict_mode_leaves_non_object_array_unchanged() {
    let mut schema = serde_json::json!({
        "type": ["string", "integer"]
    });

    patch_schema_for_strict_mode(&mut schema);

    assert_eq!(schema["type"], serde_json::json!(["string", "integer"]));
    assert!(schema.get("additionalProperties").is_none());
}

#[test]
fn strict_mode_collapses_nested_object_union_types() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "schedule": {
                "type": ["object", "string", "integer"],
                "properties": {
                    "kind": { "type": "string" },
                    "delay_ms": { "type": "integer" }
                },
                "required": ["kind"]
            },
            "payload": {
                "type": ["object", "string"],
                "properties": {
                    "kind": { "type": "string" },
                    "message": { "type": "string" }
                },
                "required": ["kind"]
            }
        },
        "required": ["schedule", "payload"]
    });

    patch_schema_for_strict_mode(&mut schema);

    assert_eq!(schema["type"], "object");
    assert_eq!(schema["properties"]["schedule"]["type"], "object");
    assert_eq!(
        schema["properties"]["schedule"]["additionalProperties"],
        false
    );
    assert_eq!(schema["properties"]["payload"]["type"], "object");
    assert_eq!(
        schema["properties"]["payload"]["additionalProperties"],
        false
    );

    let top_required: Vec<&str> = array_value(&schema["required"], "required")
        .iter()
        .map(|value| str_value(value, "required entry"))
        .collect();
    let properties = object_value(&schema["properties"], "properties");
    for name in &top_required {
        assert!(
            properties.contains_key(*name),
            "top-level required '{name}' missing from properties"
        );
    }
}

#[test]
fn to_openai_tools_collapses_union_types_end_to_end() {
    let tools = vec![serde_json::json!({
        "name": "cron",
        "description": "Manage cron jobs",
        "parameters": {
            "type": "object",
            "properties": {
                "action": { "type": "string" },
                "job": {
                    "type": ["object", "string"],
                    "properties": {
                        "name": { "type": "string" },
                        "schedule": {
                            "type": ["object", "string", "integer"],
                            "properties": {
                                "kind": { "type": "string" }
                            },
                            "required": ["kind"]
                        }
                    },
                    "required": ["name", "schedule"]
                }
            },
            "required": ["action", "job"]
        }
    })];

    let converted = to_openai_tools(&tools, true);
    assert_eq!(converted.len(), 1);

    let params = &converted[0]["function"]["parameters"];
    let mut type_arrays = Vec::new();
    find_type_arrays(params, "root", &mut type_arrays);
    assert!(
        type_arrays.is_empty(),
        "found type arrays after pipeline: {type_arrays:?}"
    );

    let mut orphans = Vec::new();
    find_required_orphans(params, "root", &mut orphans);
    assert!(orphans.is_empty(), "found required orphans: {orphans:?}");
}

#[test]
fn collapse_inside_any_of_variants() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "config": {
                "anyOf": [
                    { "type": ["object", "string"], "properties": { "key": { "type": "string" } }, "required": ["key"] },
                    { "type": "string" }
                ]
            }
        },
        "required": ["config"]
    });

    patch_schema_for_strict_mode(&mut schema);

    let any_of = array_value(&schema["properties"]["config"]["anyOf"], "config.anyOf");
    assert_eq!(any_of[0]["type"], "object");
    assert_eq!(any_of[0]["additionalProperties"], false);
    assert_eq!(any_of[1]["type"], "string");
}

#[test]
fn collapse_inside_one_of_variants() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "schedule": {
                "oneOf": [
                    { "type": ["object", "integer"], "properties": { "kind": { "type": "string" } }, "required": [] },
                    { "type": ["object", "string"], "properties": { "mode": { "type": "string" } }, "required": [] }
                ]
            }
        },
        "required": ["schedule"]
    });

    patch_schema_for_strict_mode(&mut schema);

    let one_of = array_value(&schema["properties"]["schedule"]["oneOf"], "schedule.oneOf");
    assert_eq!(one_of[0]["type"], "object");
    assert_eq!(one_of[1]["type"], "object");
}

#[test]
fn collapse_inside_array_items() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "rules": {
                "type": "array",
                "items": {
                    "type": ["object", "string"],
                    "properties": { "pattern": { "type": "string" } },
                    "required": ["pattern"]
                }
            }
        },
        "required": ["rules"]
    });

    patch_schema_for_strict_mode(&mut schema);

    let items = &schema["properties"]["rules"]["items"];
    assert_eq!(items["type"], "object");
    assert_eq!(items["additionalProperties"], false);
}

#[test]
fn collapsed_optional_object_becomes_nullable() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "config": {
                "type": ["object", "string"],
                "properties": { "key": { "type": "string" } },
                "required": ["key"]
            }
        },
        "required": ["name"]
    });

    patch_schema_for_strict_mode(&mut schema);

    let config_type = &schema["properties"]["config"]["type"];
    let arr = array_value(
        config_type,
        "optional union-type object should have array type after nullable conversion",
    );
    assert!(arr.contains(&serde_json::json!("object")));
    assert!(arr.contains(&serde_json::json!("null")));
}

#[test]
fn object_not_first_in_array_still_collapses() {
    let mut schema = serde_json::json!({
        "type": ["string", "integer", "object"],
        "properties": { "id": { "type": "string" } },
        "required": []
    });

    patch_schema_for_strict_mode(&mut schema);

    assert_eq!(schema["type"], "object");
    assert_eq!(schema["additionalProperties"], false);
}

#[test]
fn empty_type_array_is_not_object() {
    let mut schema = serde_json::json!({
        "type": []
    });

    patch_schema_for_strict_mode(&mut schema);

    assert_eq!(schema["type"], serde_json::json!([]));
    assert!(schema.get("additionalProperties").is_none());
}

fn find_type_arrays(schema: &serde_json::Value, path: &str, results: &mut Vec<String>) {
    let Some(obj) = schema.as_object() else {
        return;
    };

    if let Some(arr) = obj.get("type").and_then(|value| value.as_array()) {
        results.push(format!("{path}: type={arr:?}"));
    }
    if let Some(props) = obj.get("properties").and_then(|value| value.as_object()) {
        for (key, value) in props {
            find_type_arrays(value, &format!("{path}.{key}"), results);
        }
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(variants) = obj.get(key).and_then(|value| value.as_array()) {
            for (index, variant) in variants.iter().enumerate() {
                find_type_arrays(variant, &format!("{path}.{key}[{index}]"), results);
            }
        }
    }
    if let Some(items) = obj.get("items") {
        find_type_arrays(items, &format!("{path}.items"), results);
    }
}

fn find_required_orphans(schema: &serde_json::Value, path: &str, results: &mut Vec<String>) {
    let Some(obj) = schema.as_object() else {
        return;
    };

    if let (Some(required), Some(properties)) = (
        obj.get("required").and_then(|value| value.as_array()),
        obj.get("properties").and_then(|value| value.as_object()),
    ) {
        for entry in required {
            if let Some(name) = entry.as_str()
                && !properties.contains_key(name)
            {
                results.push(format!("{path}: required '{name}' not in properties"));
            }
        }
    }
    if let Some(props) = obj.get("properties").and_then(|value| value.as_object()) {
        for (key, value) in props {
            find_required_orphans(value, &format!("{path}.{key}"), results);
        }
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(variants) = obj.get(key).and_then(|value| value.as_array()) {
            for (index, variant) in variants.iter().enumerate() {
                find_required_orphans(variant, &format!("{path}.{key}[{index}]"), results);
            }
        }
    }
    if let Some(items) = obj.get("items") {
        find_required_orphans(items, &format!("{path}.items"), results);
    }
}
