use super::{
    parse_responses_completion, parse_tool_calls, sanitize_schema_for_openai_compat,
    to_openai_tools, to_responses_api_tools,
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
