use moltis_agents::model::StreamEvent;

use super::{
    SseLineResult, StreamingToolState, parse_responses_completion, parse_tool_calls,
    process_openai_sse_line, sanitize_schema_for_openai_compat,
    strict_mode::patch_schema_for_strict_mode, to_openai_tools, to_responses_api_tools,
};

#[test]
fn parse_tool_calls_preserves_native_falsy_types() {
    let msg = serde_json::json!({
        "tool_calls": [{
            "id": "call_1",
            "function": {
                "name": "grep",
                "arguments": {
                    "offset": 0,
                    "multiline": false,
                    "type": null
                }
            }
        }]
    });

    let calls = parse_tool_calls(&msg);

    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].arguments["offset"], 0);
    assert_eq!(calls[0].arguments["multiline"], false);
    assert!(calls[0].arguments["type"].is_null());
}

#[test]
fn parse_tool_calls_preserve_issue_693_examples() {
    let msg = serde_json::json!({
        "tool_calls": [
            {
                "id": "call_exec",
                "function": {
                    "name": "exec",
                    "arguments": {
                        "command": "echo hello",
                        "timeout": 0
                    }
                }
            },
            {
                "id": "call_edit",
                "function": {
                    "name": "Edit",
                    "arguments": {
                        "replace_all": false
                    }
                }
            },
            {
                "id": "call_grep",
                "function": {
                    "name": "Grep",
                    "arguments": {
                        "offset": 0,
                        "multiline": false,
                        "type": null
                    }
                }
            }
        ]
    });

    let calls = parse_tool_calls(&msg);

    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].arguments["timeout"], 0);
    assert_eq!(calls[1].arguments["replace_all"], false);
    assert_eq!(calls[2].arguments["offset"], 0);
    assert_eq!(calls[2].arguments["multiline"], false);
    assert!(calls[2].arguments["type"].is_null());
}

#[test]
fn parse_responses_completion_preserves_native_falsy_types() {
    let resp = serde_json::json!({
        "output": [{
            "type": "function_call",
            "call_id": "call_abc",
            "name": "grep",
            "arguments": {
                "offset": 0,
                "multiline": false,
                "type": null
            }
        }],
        "usage": {"input_tokens": 20, "output_tokens": 10}
    });

    let result = parse_responses_completion(&resp);

    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].arguments["offset"], 0);
    assert_eq!(result.tool_calls[0].arguments["multiline"], false);
    assert!(result.tool_calls[0].arguments["type"].is_null());
}

#[test]
fn responses_tools_strip_nested_not_schemas() {
    let tools = vec![serde_json::json!({
        "name": "mcp__attio__list-attribute-definitions",
        "description": "Attio test tool",
        "parameters": {
            "type": "object",
            "properties": {
                "query": {
                    "anyOf": [
                        {
                            "anyOf": [
                                {
                                    "not": {
                                        "const": ""
                                    }
                                },
                                {
                                    "type": "string"
                                }
                            ]
                        },
                        {
                            "type": "null"
                        }
                    ]
                }
            }
        }
    })];

    let converted = to_responses_api_tools(&tools);
    let params = &converted[0]["parameters"];
    let encoded = params.to_string();

    assert_eq!(converted[0]["strict"], true);
    assert!(!encoded.contains("\"not\""));
    assert_eq!(params["type"], "object");
    assert_eq!(params["additionalProperties"], false);
    assert_eq!(params["required"], serde_json::json!(["query"]));
}

#[test]
fn sanitize_schema_for_openai_compat_strips_recursive_unsupported_keywords() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "config": {
                "type": "object",
                "properties": {
                    "mode": { "type": "string" }
                },
                "if": {
                    "required": ["mode"]
                },
                "then": {
                    "properties": {
                        "enabled": { "type": "boolean" }
                    }
                },
                "else": {
                    "properties": {
                        "enabled": { "type": "boolean" }
                    }
                },
                "dependentSchemas": {
                    "mode": {
                        "properties": {
                            "extra": { "type": "string" }
                        }
                    }
                },
                "patternProperties": {
                    "^x-": { "type": "string" }
                },
                "dependentRequired": {
                    "mode": ["enabled"]
                },
                "unevaluatedProperties": false,
                "unevaluatedItems": false,
                "propertyNames": {
                    "minLength": 1
                },
                "contains": {
                    "type": "string"
                },
                "minContains": 1,
                "maxContains": 2,
                "minProperties": 1,
                "maxProperties": 4,
                "const": "active",
                "x-custom": "remove-me",
                "items": {
                    "not": {
                        "type": "integer"
                    }
                }
            }
        }
    });

    sanitize_schema_for_openai_compat(&mut schema);
    let encoded = schema.to_string();

    for keyword in [
        "\"if\"",
        "\"then\"",
        "\"else\"",
        "\"dependentSchemas\"",
        "\"patternProperties\"",
        "\"dependentRequired\"",
        "\"unevaluatedProperties\"",
        "\"unevaluatedItems\"",
        "\"propertyNames\"",
        "\"contains\"",
        "\"minContains\"",
        "\"maxContains\"",
        "\"minProperties\"",
        "\"maxProperties\"",
        "\"not\"",
        "\"x-custom\"",
    ] {
        assert!(!encoded.contains(keyword), "{keyword} should be removed");
    }
    assert_eq!(
        schema["properties"]["config"]["enum"],
        serde_json::json!(["active"])
    );
    assert_eq!(
        schema["properties"]["config"]["properties"]["mode"]["type"],
        "string"
    );
}

#[test]
fn sanitize_schema_for_openai_compat_recurses_into_array_form_items() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "tuple": {
                "type": "array",
                "items": [
                    {
                        "type": "string",
                        "not": { "const": "" }
                    },
                    {
                        "type": "object",
                        "patternProperties": {
                            "^x-": { "type": "string" }
                        }
                    }
                ]
            }
        }
    });

    sanitize_schema_for_openai_compat(&mut schema);

    let Some(tuple_items) = schema["properties"]["tuple"]["items"].as_array() else {
        panic!("tuple items should remain an array");
    };
    assert!(tuple_items[0].get("not").is_none());
    assert!(tuple_items[1].get("patternProperties").is_none());
}

#[test]
fn to_openai_tools_strict_mode_applied_by_default() {
    let tools = vec![serde_json::json!({
        "name": "create_file",
        "description": "Create a file",
        "parameters": {
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" },
                "overwrite": { "type": "boolean" }
            },
            "required": ["path"]
        }
    })];

    let converted = to_openai_tools(&tools, true);
    assert_eq!(converted.len(), 1);

    let func = &converted[0]["function"];
    assert_eq!(func["strict"], true);
    assert_eq!(func["parameters"]["additionalProperties"], false);

    let Some(required) = func["parameters"]["required"].as_array() else {
        panic!("required should be an array");
    };
    let required_names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(required_names.contains(&"path"));
    assert!(required_names.contains(&"content"));
    assert!(required_names.contains(&"overwrite"));
}

#[test]
fn to_openai_tools_non_strict_skips_patching() {
    let tools = vec![serde_json::json!({
        "name": "create_file",
        "description": "Create a file",
        "parameters": {
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" },
                "overwrite": { "type": "boolean" }
            },
            "required": ["path"]
        }
    })];

    let converted = to_openai_tools(&tools, false);
    assert_eq!(converted.len(), 1);

    let func = &converted[0]["function"];
    assert_eq!(func["strict"], false);

    let serialized = func["parameters"].to_string();
    assert!(
        !serialized.contains("additionalProperties"),
        "strict mode should not inject additionalProperties: {serialized}"
    );
    assert!(
        !serialized.contains("[\"boolean\""),
        "strict mode should not produce array-form types: {serialized}"
    );
    assert!(
        !serialized.contains("[\"string\""),
        "strict mode should not produce array-form types: {serialized}"
    );
}

#[test]
fn to_openai_tools_non_strict_complex_cron_like_schema() {
    let tools = vec![serde_json::json!({
        "name": "schedule_cron",
        "description": "Schedule a cron job",
        "parameters": {
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "job": {
                    "type": "object",
                    "properties": {
                        "enabled": { "type": "boolean" },
                        "schedule": { "type": "string" },
                        "retry": { "type": "boolean" },
                        "mode": {
                            "type": "string",
                            "enum": ["once", "recurring"]
                        },
                        "config": {
                            "type": "object",
                            "properties": {
                                "timeout": { "type": "integer" },
                                "verbose": { "type": "boolean" }
                            },
                            "required": ["timeout"]
                        }
                    },
                    "required": ["schedule"]
                }
            },
            "required": ["name", "job"]
        }
    })];

    let converted = to_openai_tools(&tools, false);
    let func = &converted[0]["function"];
    assert_eq!(func["strict"], false);

    let serialized = func["parameters"].to_string();
    assert!(
        !serialized.contains("[\"boolean\""),
        "should not contain array-form types: {serialized}"
    );
    assert!(
        !serialized.contains("[\"string\""),
        "should not contain array-form types: {serialized}"
    );
    assert!(
        !serialized.contains("[\"integer\""),
        "should not contain array-form types: {serialized}"
    );

    let Some(job_required) = func["parameters"]["properties"]["job"]["required"].as_array() else {
        panic!("job required should be an array");
    };
    assert_eq!(job_required.len(), 1);
    assert_eq!(job_required[0], "schedule");

    let Some(config_required) =
        func["parameters"]["properties"]["job"]["properties"]["config"]["required"].as_array()
    else {
        panic!("config required should be an array");
    };
    assert_eq!(config_required.len(), 1);
    assert_eq!(config_required[0], "timeout");
}

/// Issue #712: optional enum properties must include `null` in the enum
/// array when strict mode makes them nullable, otherwise the LLM sends
/// the literal string `"null"` instead of JSON null.
#[test]
fn strict_mode_nullable_enum_includes_null_in_enum_values() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "query": { "type": "string" },
            "time_range": {
                "type": "string",
                "enum": ["day", "week", "month", "year"],
                "description": "Time range filter"
            },
            "country": {
                "type": "string",
                "enum": ["US", "UK", "FR", "DE"]
            }
        },
        "required": ["query"]
    });

    patch_schema_for_strict_mode(&mut schema);

    // time_range and country are originally optional, so strict mode makes
    // them nullable. The enum array must include null alongside the original
    // string values.
    let time_range = &schema["properties"]["time_range"];
    let Some(time_enum) = time_range["enum"].as_array() else {
        panic!("time_range should have enum");
    };
    assert!(
        time_enum.iter().any(|v| v.is_null()),
        "time_range enum should include null, got: {time_enum:?}"
    );
    assert_eq!(time_enum.len(), 5, "original 4 values + null");

    let country = &schema["properties"]["country"];
    let Some(country_enum) = country["enum"].as_array() else {
        panic!("country should have enum");
    };
    assert!(
        country_enum.iter().any(|v| v.is_null()),
        "country enum should include null, got: {country_enum:?}"
    );

    // query is originally required — its enum (if any) should NOT get null injected
    assert!(schema["properties"]["query"]["enum"].is_null());
}

/// Issue #712: required enum properties should NOT get null added to
/// their enum values, even though strict mode still processes them.
#[test]
fn strict_mode_required_enum_keeps_original_values() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "mode": {
                "type": "string",
                "enum": ["search", "lookup"]
            }
        },
        "required": ["mode"]
    });

    patch_schema_for_strict_mode(&mut schema);

    let Some(mode_enum) = schema["properties"]["mode"]["enum"].as_array() else {
        panic!("mode should have enum");
    };
    assert_eq!(
        mode_enum.len(),
        2,
        "required enum should keep original values only"
    );
    assert!(
        !mode_enum.iter().any(|v| v.is_null()),
        "required enum should not include null"
    );
}

/// Issue #712: end-to-end test through `to_openai_tools` with an MCP-style
/// schema that has optional enum parameters.
#[test]
fn to_openai_tools_strict_nullable_enum_has_null() {
    let tools = vec![serde_json::json!({
        "name": "mcp__tavily__search",
        "description": "Search the web",
        "parameters": {
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "time_range": {
                    "type": "string",
                    "enum": ["day", "week", "month", "year"]
                }
            },
            "required": ["query"]
        }
    })];

    let converted = to_openai_tools(&tools, true);
    let params = &converted[0]["function"]["parameters"];

    let Some(time_enum) = params["properties"]["time_range"]["enum"].as_array() else {
        panic!("time_range should have enum");
    };
    assert!(
        time_enum.iter().any(|v| v.is_null()),
        "time_range enum should include null after strict-mode patching, got: {time_enum:?}"
    );
}

/// Fireworks regression: canonicalization strips `"type": "string"` from
/// enum properties when all enum values are strings. The post-canonicalization
/// `RestoreEnumTypeTransform` must re-infer and restore the type annotation
/// so providers like Fireworks AI don't reject the schema with 400.
#[test]
fn sanitize_restores_type_on_string_enum() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["navigate", "click", "scroll"]
            }
        },
        "required": ["action"]
    });

    sanitize_schema_for_openai_compat(&mut schema);

    assert_eq!(
        schema["properties"]["action"]["type"], "string",
        "type must be restored after canonicalization strips it"
    );
    let Some(enum_values) = schema["properties"]["action"]["enum"].as_array() else {
        panic!("enum should be preserved");
    };
    assert_eq!(enum_values.len(), 3);
}

/// Fireworks regression: `"type": "boolean"` gets canonicalized to
/// `"enum": [false, true]` (lower_boolean_and_null_types). The restore
/// transform must re-add `"type": "boolean"`.
#[test]
fn sanitize_restores_type_on_boolean_enum() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "verbose": { "type": "boolean" }
        }
    });

    sanitize_schema_for_openai_compat(&mut schema);

    assert_eq!(
        schema["properties"]["verbose"]["type"], "boolean",
        "type must be restored for boolean enums"
    );
}

/// Fireworks regression: integer enum values (e.g. priority levels) must
/// also get their type restored after canonicalization.
#[test]
fn sanitize_restores_type_on_integer_enum() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "priority": { "type": "integer", "enum": [1, 2, 3] }
        }
    });

    sanitize_schema_for_openai_compat(&mut schema);

    assert_eq!(
        schema["properties"]["priority"]["type"], "integer",
        "type must be restored for integer enums"
    );
}

/// Mixed integer+float enum values should infer "number" since JSON Schema
/// "number" subsumes "integer".
#[test]
fn sanitize_infers_number_for_mixed_int_float_enum() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "threshold": {
                "enum": [1, 1.5, 2]
            }
        }
    });

    sanitize_schema_for_openai_compat(&mut schema);

    assert_eq!(
        schema["properties"]["threshold"]["type"], "number",
        "mixed integer+float enum should infer number"
    );
}

/// End-to-end: `to_openai_tools` with strict=true must preserve type
/// annotations on enum properties after the full pipeline.
#[test]
fn to_openai_tools_strict_preserves_enum_type_annotation() {
    let tools = vec![serde_json::json!({
        "name": "browser_action",
        "description": "Perform a browser action",
        "parameters": {
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["navigate", "click", "scroll"]
                },
                "enabled": { "type": "boolean" }
            },
            "required": ["action"]
        }
    })];

    let converted = to_openai_tools(&tools, true);
    let params = &converted[0]["function"]["parameters"];

    assert_eq!(
        params["properties"]["action"]["type"], "string",
        "string enum must retain type through strict pipeline"
    );
    // `enabled` is optional → strict mode makes it nullable → type becomes
    // ["boolean", "null"]. The important thing is that "type" is present
    // (not stripped), and includes "boolean".
    let enabled_type = &params["properties"]["enabled"]["type"];
    let has_boolean = if let Some(arr) = enabled_type.as_array() {
        arr.iter().any(|v| v.as_str() == Some("boolean"))
    } else {
        enabled_type.as_str() == Some("boolean")
    };
    assert!(
        has_boolean,
        "boolean must retain type through strict pipeline, got: {enabled_type}"
    );
}

/// Mixed enum values (e.g. string + integer) should NOT get a type
/// inferred, since there's no single type that covers all values.
#[test]
fn sanitize_does_not_infer_type_for_mixed_enums() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "value": {
                "enum": ["auto", 42]
            }
        }
    });

    sanitize_schema_for_openai_compat(&mut schema);

    // Mixed types → no single type can be inferred
    assert!(
        schema["properties"]["value"]["type"].is_null(),
        "mixed enum should not get a type annotation"
    );
}

/// Issue #747: MCP tools (e.g. Home Assistant) may have `required` entries
/// referencing properties not defined in `properties`. Canonicalization adds
/// implicit `{}` schemas for these, but the prune transform removes them from
/// `required` since Gemini rejects properties without usable type info.
#[test]
fn sanitize_prunes_orphaned_required_entries() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "entity_id": { "type": "string" },
            "brightness": { "type": "integer" }
        },
        "required": ["entity_id", "brightness", "color_temp", "transition"]
    });

    sanitize_schema_for_openai_compat(&mut schema);

    let required = schema["required"]
        .as_array()
        .unwrap_or_else(|| panic!("required should be an array"));
    let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        names.contains(&"entity_id"),
        "defined property must stay in required"
    );
    assert!(
        names.contains(&"brightness"),
        "defined property must stay in required"
    );
    assert!(
        !names.contains(&"color_temp"),
        "orphaned property must be pruned from required"
    );
    assert!(
        !names.contains(&"transition"),
        "orphaned property must be pruned from required"
    );
    assert_eq!(names.len(), 2);
}

/// Issue #747: orphaned `required` entries in nested object schemas must
/// also be pruned (e.g. MCP tools with deeply nested parameters).
#[test]
fn sanitize_prunes_orphaned_required_in_nested_objects() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "target": {
                "type": "object",
                "properties": {
                    "entity_id": { "type": "string" }
                },
                "required": ["entity_id", "area_id", "device_id"]
            }
        },
        "required": ["target"]
    });

    sanitize_schema_for_openai_compat(&mut schema);

    let nested_required = schema["properties"]["target"]["required"]
        .as_array()
        .unwrap_or_else(|| panic!("nested required should be an array"));
    let names: Vec<&str> = nested_required.iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(
        names,
        vec!["entity_id"],
        "only entity_id should remain — area_id and device_id had no real schema"
    );
}

/// Issue #747: end-to-end through `to_openai_tools` with strict=false
/// (OpenRouter → Gemini path). Orphaned required entries must be pruned
/// so Gemini doesn't reject with "property is not defined".
#[test]
fn to_openai_tools_non_strict_prunes_orphaned_required() {
    let tools = vec![serde_json::json!({
        "name": "mcp__ha__light_turn_on",
        "description": "Turn on a light",
        "parameters": {
            "type": "object",
            "properties": {
                "entity_id": { "type": "string" },
                "brightness": { "type": "integer" }
            },
            "required": ["entity_id", "color_temp", "transition"]
        }
    })];

    let converted = to_openai_tools(&tools, false);
    let params = &converted[0]["function"]["parameters"];

    let required = params["required"]
        .as_array()
        .unwrap_or_else(|| panic!("required should be an array"));
    let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(
        names,
        vec!["entity_id"],
        "only defined properties should remain in required"
    );
}

/// Issue #747: schemas where `required` references properties only defined
/// through `dependentSchemas` — after canonicalization and keyword stripping,
/// those properties lack usable schemas and are pruned from `required`.
/// The `dependentSchemas` keyword itself must also be stripped.
#[test]
fn sanitize_prunes_required_from_stripped_dependent_schemas() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "mode": { "type": "string" }
        },
        "dependentSchemas": {
            "mode": {
                "properties": {
                    "extra_param": { "type": "string" }
                }
            }
        },
        "required": ["mode", "extra_param"]
    });

    sanitize_schema_for_openai_compat(&mut schema);

    let required = schema["required"]
        .as_array()
        .unwrap_or_else(|| panic!("required should be an array"));
    let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(names.contains(&"mode"), "mode should stay in required");
    assert!(
        !names.contains(&"extra_param"),
        "extra_param should be pruned — only defined via dependentSchemas"
    );
    // `dependentSchemas` itself must be stripped
    assert!(
        schema.get("dependentSchemas").is_none(),
        "dependentSchemas should be stripped"
    );
}

/// Issue #743: MCP tools declaring `$schema: "http://json-schema.org/draft-07/schema#"`
/// (e.g. Attio) bypass canonicalization entirely because `SchemaDocument::from_json()`
/// only accepts Draft 2020-12. Without canonicalization, stripping `not` leaves
/// empty `{}` schemas inside `anyOf` which OpenAI rejects ("schema must have a
/// 'type' key"). The sanitizer must strip `$schema` before canonicalization so
/// that draft-07 schemas get the same AST resolution as draft 2020-12.
#[test]
fn sanitize_draft07_schema_strips_unsupported_keywords() {
    // Reproduces the Attio MCP schema pattern from issue #743:
    // nested anyOf with `not` and no `type` key, declared as draft-07.
    let mut schema = serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "properties": {
            "query": {
                "anyOf": [
                    {
                        "anyOf": [
                            {
                                "not": {
                                    "const": ""
                                }
                            },
                            {
                                "type": "string"
                            }
                        ]
                    },
                    {
                        "type": "null"
                    }
                ]
            }
        }
    });

    sanitize_schema_for_openai_compat(&mut schema);
    let encoded = schema.to_string();

    // `$schema` must be stripped.
    assert!(
        !encoded.contains("$schema"),
        "$schema should be removed, got: {encoded}"
    );
    // `not` must be stripped.
    assert!(
        !encoded.contains("\"not\""),
        "\"not\" should be removed from draft-07 schema, got: {encoded}"
    );

    // Critical assertion: no empty `{}` schemas left inside anyOf variants.
    // Without canonicalization, stripping `not` leaves `{}` which OpenAI
    // rejects with "schema must have a 'type' key".
    assert!(
        !encoded.contains("{}"),
        "empty schemas should not remain after sanitization, got: {encoded}"
    );
    // The root `type: object` must survive.
    assert_eq!(schema["type"], "object");
}

/// Issue #743: end-to-end through `to_responses_api_tools` with a draft-07
/// schema containing the exact Attio pattern that OpenAI rejects.
#[test]
fn responses_tools_draft07_attio_schema_normalized() {
    let tools = vec![serde_json::json!({
        "name": "mcp__attio__list-attribute-definitions",
        "description": "Attio test tool (draft-07)",
        "parameters": {
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "properties": {
                "query": {
                    "anyOf": [
                        {
                            "anyOf": [
                                {
                                    "not": {
                                        "const": ""
                                    }
                                },
                                {
                                    "type": "string"
                                }
                            ]
                        },
                        {
                            "type": "null"
                        }
                    ]
                }
            }
        }
    })];

    let converted = to_responses_api_tools(&tools);
    let params = &converted[0]["parameters"];
    let encoded = params.to_string();

    assert!(
        !encoded.contains("$schema"),
        "$schema must be removed: {encoded}"
    );
    assert!(
        !encoded.contains("\"not\""),
        "\"not\" must be removed: {encoded}"
    );
    assert!(
        !encoded.contains("{}"),
        "empty schemas should not remain after sanitization: {encoded}"
    );
    assert_eq!(params["type"], "object");
}

/// Issue #760: draft-07 schemas with `$schema` inside nested `definitions`
/// must go through full canonicalization, not fall back to best-effort
/// normalization (which logs a WARN on every call). The `$schema` stripping
/// must be recursive (not root-only) so that `validate_schema_dialects`
/// inside `json_schema_ast` doesn't reject the schema at a nested pointer.
///
/// We detect the fallback path by checking for a canonicalization side-effect:
/// `lower_boolean_and_null_types` converts `type: "boolean"` to `enum:
/// [false, true]`, which only happens during canonicalization.
#[test]
fn sanitize_draft07_nested_definitions_schema_canonicalized() {
    let mut schema = serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "definitions": {
            "Threshold": {
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean" },
                    "value": { "type": "integer" }
                },
                "required": ["enabled", "value"]
            }
        },
        "properties": {
            "name": { "type": "string" },
            "verbose": { "type": "boolean" },
            "threshold": { "$ref": "#/definitions/Threshold" }
        },
        "required": ["name"]
    });

    sanitize_schema_for_openai_compat(&mut schema);
    let encoded = schema.to_string();

    // `$schema` must be stripped at all levels.
    assert!(
        !encoded.contains("$schema"),
        "$schema should be removed from all levels, got: {encoded}"
    );
    // Root type preserved.
    assert_eq!(schema["type"], "object");
    // `name` property type preserved.
    assert_eq!(schema["properties"]["name"]["type"], "string");

    // Canonicalization lowers `type: "boolean"` → `enum: [false, true]`.
    // If this enum is present, canonicalization ran (not the fallback path
    // that would emit a WARN log).
    let verbose_enum = schema["properties"]["verbose"]["enum"].as_array();
    assert!(
        verbose_enum.is_some(),
        "boolean property should have enum after canonicalization (proves no WARN fallback), got: {}",
        schema["properties"]["verbose"]
    );
}

/// Issue #760: draft-07 schemas must go through full canonicalization (not
/// just the best-effort fallback). Canonicalization lowers `type: "boolean"`
/// to `enum: [false, true]` — if this enum is present after sanitization,
/// it proves the schema was canonicalized rather than falling through to
/// the raw-schema fallback path.
#[test]
fn sanitize_draft07_schema_uses_canonicalization_not_fallback() {
    let mut schema = serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "properties": {
            "verbose": { "type": "boolean" },
            "name": { "type": "string" }
        }
    });

    sanitize_schema_for_openai_compat(&mut schema);

    // Canonicalization lowers `type: "boolean"` → `enum: [false, true]`,
    // then `RestoreEnumTypeTransform` re-adds `type: "boolean"`. The
    // presence of `enum` proves canonicalization ran (the fallback path
    // would leave `type: "boolean"` unchanged without an `enum`).
    assert_eq!(
        schema["properties"]["verbose"]["type"], "boolean",
        "type must be restored after canonicalization"
    );
    let Some(verbose_enum) = schema["properties"]["verbose"]["enum"].as_array() else {
        panic!(
            "boolean property should have enum after canonicalization (proves non-fallback path), got: {}",
            schema["properties"]["verbose"]
        );
    };
    assert_eq!(
        verbose_enum.len(),
        2,
        "boolean enum should have [false, true]"
    );
}

/// `has_usable_type` considers bare `true`, empty `{}`, and description-only
/// schemas as NOT having a usable type.  Properties that were genuinely
/// defined (with `type`, `enum`, etc.) survive.
#[test]
fn schema_normalization_prunes_orphaned_required_with_stricter_check() {
    // This tests the strengthened `has_usable_type` check.
    // `area_id` has no definition at all (canonicalized to `true`), while
    // `entity_id` has a real type — only `entity_id` should remain required.
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "entity_id": { "type": "string" },
            "brightness": { "type": "integer" }
        },
        "required": ["entity_id", "brightness", "orphan"]
    });

    sanitize_schema_for_openai_compat(&mut schema);

    let required = schema["required"]
        .as_array()
        .unwrap_or_else(|| panic!("required should be an array"));
    let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(names.contains(&"entity_id"), "entity_id should stay");
    assert!(names.contains(&"brightness"), "brightness should stay");
    assert!(
        !names.contains(&"orphan"),
        "orphan property with no definition should be pruned"
    );
}

/// The strengthened `has_usable_type` check rejects description-only and
/// empty-object properties that were previously considered "defined".
/// This test verifies the end-to-end pruning behavior.
#[test]
fn schema_normalization_prunes_description_only_from_required() {
    // `description`-only property gets canonicalized to `true`, then the
    // stricter `has_usable_type` check in `PruneOrphanedRequiredTransform`
    // drops it from `required`.
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "query": { "type": "string" },
            "context": { "description": "Search context" }
        },
        "required": ["query", "context"]
    });

    sanitize_schema_for_openai_compat(&mut schema);

    let required = schema["required"]
        .as_array()
        .unwrap_or_else(|| panic!("required should be an array"));
    let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(names.contains(&"query"), "typed property stays in required");
    assert!(
        !names.contains(&"context"),
        "description-only property should be pruned from required"
    );
}

// ── Streaming metadata extraction ──────────────────────────────────

/// SSE chunk with `thought_signature` emits it in ToolCallStart metadata.
#[test]
fn streaming_tool_call_start_extracts_thought_signature() {
    let data = serde_json::json!({
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": "call_1",
                    "thought_signature": "sig_xyz",
                    "function": { "name": "get_weather", "arguments": "" }
                }]
            }
        }]
    })
    .to_string();

    let mut state = StreamingToolState::default();
    let result = process_openai_sse_line(&data, &mut state);
    let SseLineResult::Events(events) = result else {
        panic!("expected Events");
    };
    let found = events
        .iter()
        .any(|e| matches!(e, StreamEvent::ToolCallStart { metadata, .. } if metadata.as_ref().is_some_and(|m| m["thought_signature"] == "sig_xyz")));
    assert!(
        found,
        "should emit ToolCallStart with thought_signature metadata"
    );
}

/// SSE chunk without `thought_signature` has None metadata.
#[test]
fn streaming_tool_call_start_no_metadata_when_absent() {
    let data = serde_json::json!({
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": "call_1",
                    "function": { "name": "exec", "arguments": "" }
                }]
            }
        }]
    })
    .to_string();

    let mut state = StreamingToolState::default();
    let result = process_openai_sse_line(&data, &mut state);
    let SseLineResult::Events(events) = result else {
        panic!("expected Events");
    };
    let found = events
        .iter()
        .any(|e| matches!(e, StreamEvent::ToolCallStart { metadata: None, .. }));
    assert!(found, "ToolCallStart should have None metadata");
}

/// Non-streaming `parse_tool_calls` extracts metadata.
#[test]
fn parse_tool_calls_extracts_metadata() {
    let message = serde_json::json!({
        "tool_calls": [{
            "id": "call_1",
            "thought_signature": "sig_abc",
            "function": { "name": "exec", "arguments": "{}" }
        }]
    });

    let tool_calls = parse_tool_calls(&message);
    assert_eq!(tool_calls.len(), 1);
    assert!(
        tool_calls[0]
            .metadata
            .as_ref()
            .is_some_and(|m| m["thought_signature"] == "sig_abc"),
        "should extract thought_signature into metadata"
    );
}

/// Issue #712: enum-only schemas (no `type` key) must also get null
/// appended. Canonicalization can strip the redundant `"type": "string"`
/// leaving just `{"enum": [...]}` — the final fallback in make_nullable
/// must handle this.
#[test]
fn strict_mode_nullable_enum_only_schema_includes_null() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "query": { "type": "string" },
            "time_range": {
                "enum": ["day", "week", "month", "year"],
                "description": "No type key — enum-only schema"
            }
        },
        "required": ["query"]
    });

    patch_schema_for_strict_mode(&mut schema);

    let Some(time_enum) = schema["properties"]["time_range"]["enum"].as_array() else {
        panic!("time_range should have enum");
    };
    assert!(
        time_enum.iter().any(|v| v.is_null()),
        "enum-only schema should include null, got: {time_enum:?}"
    );
}
