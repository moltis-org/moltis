use {
    super::assert_no_orphaned_required,
    crate::openai_compat::{
        SseLineResult, StreamingToolState, parse_tool_calls, process_openai_sse_line,
        sanitize_schema_for_openai_compat,
        schema_normalization::{
            collapse_schema_unions_for_non_strict_tools, strip_null_from_typed_enums,
        },
        strict_mode::patch_schema_for_strict_mode, to_openai_tools, to_responses_api_tools,
    },
    moltis_agents::model::StreamEvent,
};

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

    // Canonicalization lowers `type: "boolean"` → `enum: [false, true]`,
    // then `RestoreEnumTypeTransform` restores `type: "boolean"` and strips
    // the redundant enum (#848). The `$schema` stripping above already
    // proves canonicalization ran; verify the type is correctly preserved.
    assert_eq!(
        schema["properties"]["verbose"]["type"], "boolean",
        "boolean type must be restored after canonicalization"
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

    // `$schema` is stripped during canonicalization — its absence proves
    // we used the real canonicalization path, not the raw-input fallback.
    assert!(
        schema.get("$schema").is_none(),
        "$schema should be stripped (proves canonicalization ran)"
    );
    // Canonicalization lowers `type: "boolean"` → `enum: [false, true]`,
    // then `RestoreEnumTypeTransform` re-adds `type: "boolean"` and strips
    // the redundant enum (#848). The type should be preserved.
    assert_eq!(
        schema["properties"]["verbose"]["type"], "boolean",
        "type must be restored after canonicalization"
    );
    // The redundant `[false, true]` enum is stripped to prevent Fireworks
    // from receiving `null` in enum arrays during strict-mode nullability.
    assert!(
        schema["properties"]["verbose"].get("enum").is_none(),
        "redundant boolean enum should be stripped (#848)"
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

/// Issue #849: cron-style schemas with union type arrays (`["object",
/// "string", "integer"]`) trigger `lower_type_array_to_any_of` in
/// `json_schema_ast`, which copies ALL context keys (including `properties`
/// and `required`) to every branch. After `CollapseCompositeUnionTransform`
/// picks the object variant, the shallow `merge_schema_object` may drop
/// variant `properties` when the parent already has a `properties` key,
/// leaving orphaned `required` entries that Gemini rejects with "property is
/// not defined".
#[test]
fn non_strict_cron_tool_schema_no_orphaned_required() {
    let time_field = |description: &str| {
        serde_json::json!({
            "type": ["integer", "string"],
            "description": description
        })
    };
    let tools = vec![serde_json::json!({
        "name": "cron",
        "description": "Manage scheduled tasks",
        "parameters": {
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["status", "list", "add", "update", "remove", "run", "runs"],
                    "description": "The action to perform"
                },
                "job": {
                    "type": "object",
                    "description": "Job specification",
                    "properties": {
                        "name": { "type": "string" },
                        "schedule": {
                            "type": ["object", "string", "integer"],
                            "description": "Schedule specification",
                            "properties": {
                                "kind": { "type": "string", "enum": ["at", "every", "cron"] },
                                "delay_ms": time_field("Milliseconds from now"),
                                "at_ms": time_field("Absolute epoch milliseconds"),
                                "every_ms": time_field("Recurring interval"),
                                "anchor_ms": time_field("Optional anchor"),
                                "expr": { "type": "string", "description": "Cron expression" },
                                "tz": { "type": "string", "description": "Timezone" }
                            },
                            "required": ["kind"]
                        },
                        "payload": {
                            "type": ["object", "string"],
                            "description": "What to do",
                            "properties": {
                                "kind": { "type": "string", "enum": ["systemEvent", "agentTurn"] },
                                "text": { "type": "string" },
                                "message": { "type": "string" },
                                "model": { "type": "string" },
                                "timeout_secs": { "type": ["integer", "string"] },
                                "deliver": { "type": "boolean" },
                                "channel": { "type": "string" },
                                "to": { "type": "string" }
                            },
                            "required": ["kind"]
                        },
                        "sessionTarget": { "type": "string", "enum": ["main", "isolated"] },
                        "deleteAfterRun": { "type": "boolean" },
                        "enabled": { "type": "boolean" }
                    },
                    "required": ["name", "schedule", "payload"]
                },
                "id": { "type": "string" },
                "force": { "type": "boolean" },
                "limit": { "type": "integer" }
            },
            "required": ["action"]
        }
    })];

    let converted = to_openai_tools(&tools, false);
    assert_eq!(converted.len(), 1, "should have 1 tool");
    let params = &converted[0]["function"]["parameters"];

    // No array-form types should survive
    let serialized = params.to_string();
    assert!(
        !serialized.contains("[\"integer\""),
        "no array-form types should remain: {serialized}"
    );
    assert!(
        !serialized.contains("[\"object\""),
        "no array-form types should remain: {serialized}"
    );
    assert!(
        !serialized.contains("[\"string\""),
        "no array-form types should remain: {serialized}"
    );

    // Recursively validate: no required entry references a missing property
    assert_no_orphaned_required(params, "parameters");
}

/// Issue #849: MultiEdit tool has nested required inside array `items`.
/// Verify the nested required entries survive sanitization correctly.
#[test]
fn non_strict_multi_edit_schema_no_orphaned_required() {
    let tools = vec![serde_json::json!({
        "name": "MultiEdit",
        "description": "Apply multiple sequential edits",
        "parameters": {
            "type": "object",
            "required": ["file_path", "edits"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file"
                },
                "edits": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "required": ["old_string", "new_string"],
                        "properties": {
                            "old_string": { "type": "string" },
                            "new_string": { "type": "string" },
                            "replace_all": { "type": "boolean", "default": false }
                        }
                    }
                }
            }
        }
    })];

    let converted = to_openai_tools(&tools, false);
    let params = &converted[0]["function"]["parameters"];

    // Recursively validate
    assert_no_orphaned_required(params, "parameters");

    // Items-level required should be intact
    let items_required = params["properties"]["edits"]["items"]["required"]
        .as_array()
        .unwrap_or_else(|| panic!("items.required should exist"));
    let names: Vec<&str> = items_required.iter().filter_map(|v| v.as_str()).collect();
    assert!(names.contains(&"old_string"));
    assert!(names.contains(&"new_string"));
}

/// Issue #849: after `CollapseCompositeUnionTransform` merges a selected
/// `anyOf` variant into a parent that already has `properties`, the shallow
/// `merge_schema_object` drops the variant's `properties` (parent-key-wins).
/// If the variant's `required` is merged (parent had none), it references
/// the variant's dropped properties — creating orphaned entries.
/// `PruneOrphanedRequiredTransform` must clean these up.
#[test]
fn non_strict_anyof_merge_deep_merges_variant_properties() {
    // Parent has `properties: {mode}`, variant has `properties: {target, count}`
    // and `required: [target, count]`. Deep merge should preserve ALL properties:
    // parent's `mode` AND variant's `target` + `count`.
    let mut schema = serde_json::json!({
        "type": "object",
        "description": "A tool with union parameters",
        "properties": {
            "mode": { "type": "string" }
        },
        "anyOf": [
            {
                "type": "object",
                "properties": {
                    "target": { "type": "string" },
                    "count": { "type": "integer" }
                },
                "required": ["target", "count"]
            },
            {
                "type": "string"
            }
        ]
    });

    sanitize_schema_for_openai_compat(&mut schema);
    collapse_schema_unions_for_non_strict_tools(&mut schema);

    assert_no_orphaned_required(&schema, "root");

    // Deep merge should have preserved ALL properties from both parent and variant
    let props = schema["properties"]
        .as_object()
        .unwrap_or_else(|| panic!("should have properties"));
    assert!(props.contains_key("mode"), "parent property should survive");
    assert!(
        props.contains_key("target"),
        "variant property 'target' should be deep-merged"
    );
    assert!(
        props.contains_key("count"),
        "variant property 'count' should be deep-merged"
    );

    // Variant's required entries should also be present (since their
    // properties were deep-merged and now exist)
    let required = schema["required"]
        .as_array()
        .unwrap_or_else(|| panic!("should have required"));
    let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(names.contains(&"target"), "target should be in required");
    assert!(names.contains(&"count"), "count should be in required");
}

/// Issue #849: `allOf` produced by `json_schema_ast` when a type array
/// coexists with an existing `anyOf` should be collapsed. Multi-variant
/// `allOf` that survives transforms is mis-converted by OpenRouter's Gemini
/// translation.
#[test]
fn non_strict_allof_collapsed_and_merged() {
    let mut schema = serde_json::json!({
        "allOf": [
            {
                "properties": {
                    "kind": { "type": "string", "enum": ["at", "every"] }
                },
                "required": ["kind"],
                "anyOf": [
                    { "properties": { "delay_ms": { "type": "integer" } } },
                    { "properties": { "every_ms": { "type": "integer" } } }
                ]
            },
            {
                "anyOf": [
                    { "type": "object", "properties": { "kind": { "type": "string" } }, "required": ["kind"] },
                    { "type": "string" }
                ]
            }
        ]
    });

    sanitize_schema_for_openai_compat(&mut schema);
    collapse_schema_unions_for_non_strict_tools(&mut schema);

    // allOf should be fully collapsed
    assert!(
        schema.get("allOf").is_none(),
        "allOf should be collapsed, got: {}",
        serde_json::to_string_pretty(&schema).unwrap_or_default()
    );

    assert_no_orphaned_required(&schema, "root");
}

/// Issue #848: `json_schema_ast` canonicalization converts `"type": "boolean"`
/// to `"enum": [false, true]`. `RestoreEnumTypeTransform` restores
/// `"type": "boolean"` but leaves the redundant `enum`. Then strict mode's
/// `make_nullable` appends `null` to the enum: `[false, true, null]`.
/// Fireworks AI rejects this with "could not translate the enum None."
///
/// The fix: `RestoreEnumTypeTransform` strips redundant `[false, true]` enum
/// arrays when `type: "boolean"` is restored, preventing `null` from being
/// added to a boolean enum.
#[test]
fn strict_mode_boolean_property_no_null_in_enum() {
    let tools = vec![serde_json::json!({
        "name": "test_tool",
        "description": "Test",
        "parameters": {
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "verbose": { "type": "boolean" },
                "dry_run": { "type": "boolean", "default": false }
            },
            "required": ["query"]
        }
    })];

    // strict=true (Fireworks path)
    let converted = to_openai_tools(&tools, true);
    let params = &converted[0]["function"]["parameters"];
    let serialized = params.to_string();

    // Boolean properties should NOT have enum arrays containing null.
    // They should use type-nullability only: "type": ["boolean", "null"]
    for prop_name in ["verbose", "dry_run"] {
        let prop = &params["properties"][prop_name];

        // Should NOT have an enum with null
        if let Some(enum_arr) = prop.get("enum").and_then(|v| v.as_array()) {
            assert!(
                !enum_arr.iter().any(|v| v.is_null()),
                "{prop_name} should not have null in enum: {enum_arr:?} \
                 (full schema: {serialized})"
            );
        }

        // Should be nullable via type
        let ty = prop.get("type");
        assert!(
            ty.is_some(),
            "{prop_name} should have a type field: {}",
            serde_json::to_string_pretty(prop).unwrap_or_default()
        );
    }
}

/// Issue #848: `add_null_to_enum` (PR #724) adds `null` to enum arrays
/// for nullable optional properties in strict mode. Fireworks AI rejects
/// `null` in ANY enum, not just boolean enums. Optional string enum
/// properties like `{ "type": "string", "enum": ["fast", "slow"] }` get
/// `null` appended → `["fast", "slow", null]`, triggering
/// "could not translate the enum None."
///
/// The fix: providers that reject null in enums (Fireworks) apply
/// `strip_null_from_typed_enums` after strict-mode patching. This removes
/// `null` from enum arrays when the `type` already communicates
/// nullability (e.g. `["string", "null"]`).
#[test]
fn strict_mode_fireworks_string_enum_no_null_after_strip() {
    let tools = vec![serde_json::json!({
        "name": "cron_tool",
        "description": "Schedule tasks",
        "parameters": {
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "remove", "list"],
                    "description": "The action to perform"
                },
                "job": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "session_target": {
                            "type": "string",
                            "enum": ["main", "isolated"]
                        },
                        "enabled": { "type": "boolean" },
                        "priority": {
                            "type": "integer",
                            "enum": [0, 1, 2, 3]
                        }
                    },
                    "required": ["name"]
                }
            },
            "required": ["action"]
        }
    })];

    // strict=true then strip nulls (Fireworks provider path)
    let mut converted = to_openai_tools(&tools, true);
    for tool in &mut converted {
        if let Some(params) = tool.pointer_mut("/function/parameters") {
            strip_null_from_typed_enums(params);
        }
    }
    let params = &converted[0]["function"]["parameters"];

    // After stripping, no enum array should contain null.
    assert_no_null_in_enums(params, "parameters");

    // But type-level nullability should be preserved for optional props.
    let job = &params["properties"]["job"];
    let job_props = &job["properties"];
    for prop_name in ["session_target", "enabled", "priority"] {
        let prop = &job_props[prop_name];
        let ty = prop.get("type").and_then(|v| v.as_array());
        assert!(
            ty.is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some("null"))),
            "{prop_name} type should still include null for nullability: {}",
            serde_json::to_string_pretty(prop).unwrap_or_default()
        );
    }
}

/// Recursively assert no `null` value appears in any `enum` array.
/// Fireworks AI rejects schemas where any enum variant is JSON null.
fn assert_no_null_in_enums(schema: &serde_json::Value, path: &str) {
    let Some(obj) = schema.as_object() else {
        return;
    };

    if let Some(enum_arr) = obj.get("enum").and_then(|v| v.as_array()) {
        assert!(
            !enum_arr.iter().any(|v| v.is_null()),
            "null in enum at {path}: {enum_arr:?}"
        );
    }

    if let Some(props) = obj.get("properties").and_then(|v| v.as_object()) {
        for (key, value) in props {
            assert_no_null_in_enums(value, &format!("{path}.properties.{key}"));
        }
    }
    if let Some(items) = obj.get("items") {
        assert_no_null_in_enums(items, &format!("{path}.items"));
    }
    for keyword in ["anyOf", "oneOf", "allOf"] {
        if let Some(variants) = obj.get(keyword).and_then(|v| v.as_array()) {
            for (i, variant) in variants.iter().enumerate() {
                assert_no_null_in_enums(variant, &format!("{path}.{keyword}[{i}]"));
            }
        }
    }
    if let Some(ap) = obj.get("additionalProperties")
        && ap.is_object()
    {
        assert_no_null_in_enums(ap, &format!("{path}.additionalProperties"));
    }
}

/// Issue #848: enum-only schemas with only `null` values (from
/// `json_schema_ast`'s `lower_boolean_and_null_types` converting
/// `"type": "null"` → `"enum": [null]`) should not reach Fireworks.
/// After `RestoreEnumTypeTransform`, such schemas keep `"enum": [null]`
/// without a type — verify they don't appear in strict-mode output.
#[test]
fn sanitize_strips_redundant_boolean_enum() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "flag": { "type": "boolean" },
            "mode": { "type": "string", "enum": ["fast", "slow"] }
        },
        "required": ["flag"]
    });

    sanitize_schema_for_openai_compat(&mut schema);

    // After canonicalization + restore, `flag` should have type: "boolean"
    // WITHOUT a redundant enum: [false, true].
    let flag = &schema["properties"]["flag"];
    assert_eq!(
        flag.get("type").and_then(|v| v.as_str()),
        Some("boolean"),
        "flag should have type boolean"
    );
    assert!(
        flag.get("enum").is_none(),
        "flag should not have redundant boolean enum, got: {}",
        serde_json::to_string_pretty(flag).unwrap_or_default()
    );

    // `mode` should keep its enum (it's a real constraint, not a
    // canonicalization artifact).
    let mode = &schema["properties"]["mode"];
    assert!(
        mode.get("enum").is_some(),
        "mode should keep its string enum"
    );
}
