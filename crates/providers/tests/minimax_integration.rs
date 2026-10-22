//! Live integration tests for the MiniMax provider.
//!
//! These tests hit the real MiniMax API and require `MINIMAX_API_KEY` in the
//! environment. They are `#[ignore]`d by default so `cargo test` skips them.
//!
//! Run with:
//!   cargo test --test minimax_integration -- --ignored
//!
//! Covers regressions for:
//! - #578: system prompt not injected (silently ignored top-level `system` field)
//! - #582: `null` for optional array tool parameters (`allow_tools`, `deny_tools`)
//! - #592: MiniMax rejects `role: system` in messages array

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use {
    futures::StreamExt,
    moltis_agents::model::{ChatMessage, LlmProvider, StreamEvent, ToolCall},
    moltis_config::schema::{ProviderEntry, ProvidersConfig},
    moltis_providers::ProviderRegistry,
    secrecy::{ExposeSecret, Secret},
};

const MINIMAX_BASE_URL: &str = "https://api.minimax.io/v1";
const TEST_MODEL: &str = "MiniMax-M3.1-Flash";
/// Exercise M3's non-streaming tool path as well as Flash's streaming path.
const TOOL_MODEL: &str = "MiniMax-M3";

/// Known MiniMax models we catalog. Keep in sync with `MINIMAX_MODELS` in
/// `crates/providers/src/model_catalogs.rs`.
const KNOWN_MODELS: &[&str] = &[TEST_MODEL, TOOL_MODEL];

fn api_key() -> Secret<String> {
    let key = std::env::var("MINIMAX_API_KEY")
        .expect("MINIMAX_API_KEY must be set for integration tests");
    Secret::new(key)
}

fn make_registry(key: Secret<String>, models: Vec<String>) -> ProviderRegistry {
    let mut config = ProvidersConfig {
        offered: vec!["minimax".into()],
        ..ProvidersConfig::default()
    };
    config.providers.insert("minimax".into(), ProviderEntry {
        api_key: Some(key),
        base_url: Some(MINIMAX_BASE_URL.into()),
        models,
        ..ProviderEntry::default()
    });
    ProviderRegistry::from_config_with_static_catalogs(&config, &HashMap::new(), HashMap::new())
}

fn make_provider(model: &str) -> Arc<dyn LlmProvider> {
    make_registry(api_key(), vec![model.into()])
        .get(&format!("minimax::{model}"))
        .expect("configured MiniMax model")
}

#[test]
fn known_models_match_production_catalog() {
    let registry = make_registry(Secret::new("test-only-key".into()), Vec::new());
    let ids: HashSet<&str> = registry
        .list_models()
        .iter()
        .filter(|model| model.provider == "minimax")
        .map(|model| model.id.strip_prefix("minimax::").unwrap_or(&model.id))
        .collect();
    assert_eq!(ids, KNOWN_MODELS.iter().copied().collect());
}

/// Tool schema in moltis-internal flat format (name, description, parameters).
/// `to_openai_tools()` wraps this into the nested OpenAI format before sending.
fn weather_tool() -> serde_json::Value {
    serde_json::json!({
        "name": "get_weather",
        "description": "Get current weather for a location. You MUST call this tool when asked about weather.",
        "parameters": {
            "type": "object",
            "properties": {
                "location": {
                    "type": "string",
                    "description": "City name"
                }
            },
            "required": ["location"]
        }
    })
}

/// spawn_agent-like tool with optional array parameters.
fn spawn_agent_tool() -> serde_json::Value {
    serde_json::json!({
        "name": "spawn_agent",
        "description": "Spawn a sub-agent to perform a task. You MUST call this tool when asked to spawn an agent.",
        "parameters": {
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Task description"
                },
                "allow_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional whitelist of tool names"
                },
                "deny_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional blacklist of tool names"
                }
            },
            "required": ["task"]
        }
    })
}

// ── #578 / #592: System prompt handling ──────────────────────────────────────

/// Regression test for #578 and #592: system prompt must reach the model.
///
/// We instruct the model via system prompt to include a specific keyword
/// in every response. If the keyword appears, the system prompt was delivered.
#[tokio::test]
#[ignore]
async fn system_prompt_is_received_by_model_non_streaming() {
    let p = make_provider(TEST_MODEL);
    let keyword = "PINEAPPLE";
    let messages = vec![
        ChatMessage::system(format!(
            "You MUST include the exact word \"{keyword}\" in every response, no matter what the user asks."
        )),
        ChatMessage::user("What is 2+2?"),
    ];

    let response = p
        .complete(&messages, &[])
        .await
        .expect("non-streaming completion should succeed");

    let text = response.text.expect("response must contain text");
    assert!(
        text.to_lowercase().contains(&keyword.to_lowercase()),
        "system prompt was not received by model: response = {text:?}"
    );
    assert!(
        response.usage.input_tokens > 0,
        "should report input tokens"
    );
    assert!(
        response.usage.output_tokens > 0,
        "should report output tokens"
    );
}

/// Streaming variant of the system prompt test.
#[tokio::test]
#[ignore]
async fn system_prompt_is_received_by_model_streaming() {
    let p = make_provider(TEST_MODEL);
    let keyword = "COCONUT";
    let messages = vec![
        ChatMessage::system(format!(
            "You MUST include the exact word \"{keyword}\" in every response, no matter what the user asks."
        )),
        ChatMessage::user("What is 3+3?"),
    ];

    let mut stream = p.stream_with_tools(messages, vec![]);
    let mut full_text = String::new();
    let mut saw_done = false;

    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::Delta(chunk) => full_text.push_str(&chunk),
            StreamEvent::Done(usage) => {
                saw_done = true;
                assert!(usage.input_tokens > 0, "should report input tokens");
                assert!(usage.output_tokens > 0, "should report output tokens");
                break;
            },
            StreamEvent::Error(err) => panic!("stream error: {err}"),
            _ => {},
        }
    }

    assert!(saw_done, "stream must emit Done event");
    assert!(
        full_text.to_lowercase().contains(&keyword.to_lowercase()),
        "system prompt was not received by model: response = {full_text:?}"
    );
}

/// Multiple system messages should all be delivered.
#[tokio::test]
#[ignore]
async fn multiple_system_messages_all_delivered() {
    let p = make_provider(TEST_MODEL);
    let messages = vec![
        ChatMessage::system("The first secret marker is the exact word ZEBRA."),
        ChatMessage::user(
            "Reply with both secret markers from your instructions. Output only the two markers.",
        ),
        ChatMessage::system("The second secret marker is the exact word GIRAFFE."),
    ];

    let response = p
        .complete(&messages, &[])
        .await
        .expect("completion should succeed");

    let text = response.text.expect("response must contain text");
    let lower_text = text.to_lowercase();
    assert!(
        lower_text.contains("zebra"),
        "first system prompt missing: {text:?}"
    );
    assert!(
        lower_text.contains("giraffe"),
        "second system prompt missing: {text:?}"
    );
}

// ── Tool calling ─────────────────────────────────────────────────────────────

/// The model must be able to call a tool successfully.
#[tokio::test]
#[ignore]
async fn tool_call_round_trip_non_streaming() {
    let p = make_provider(TOOL_MODEL);
    let tools = vec![weather_tool()];

    let messages = vec![ChatMessage::user(
        "What's the weather like in Tokyo? You must use the get_weather tool to answer.",
    )];

    let response = p
        .complete(&messages, &tools)
        .await
        .expect("completion with tools should succeed");

    assert!(
        !response.tool_calls.is_empty(),
        "model should call the get_weather tool, got text: {:?}",
        response.text
    );

    let tool_call = &response.tool_calls[0];
    assert_eq!(tool_call.name, "get_weather");
    let args = &tool_call.arguments;
    assert!(
        args.get("location").is_some(),
        "tool call should include location, got: {args}"
    );
}

/// Streaming variant: model calls a tool and we get proper streaming events.
#[tokio::test]
#[ignore]
async fn tool_call_round_trip_streaming() {
    let p = make_provider(TOOL_MODEL);
    let tools = vec![weather_tool()];

    let messages = vec![ChatMessage::user(
        "What's the weather in Paris? You must use the get_weather tool.",
    )];

    let mut stream = p.stream_with_tools(messages, tools);
    let mut saw_tool_start = false;
    let mut saw_done = false;
    let mut tool_name = String::new();

    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::ToolCallStart { name, .. } => {
                saw_tool_start = true;
                tool_name = name;
            },
            StreamEvent::Done(_) => {
                saw_done = true;
                break;
            },
            StreamEvent::Error(err) => panic!("stream error: {err}"),
            _ => {},
        }
    }

    assert!(saw_done, "stream must emit Done event");
    assert!(saw_tool_start, "stream should include a tool call");
    assert_eq!(tool_name, "get_weather");
}

/// Tool call with optional array params that might be null.
/// Regression for #582: spawn_agent-like schema with optional arrays.
#[tokio::test]
#[ignore]
async fn tool_call_with_optional_array_params() {
    let p = make_provider(TOOL_MODEL);
    let tools = vec![spawn_agent_tool()];

    let messages = vec![ChatMessage::user(
        "Use the spawn_agent tool to analyze the current directory. Set the task to 'list files'.",
    )];

    let response = p
        .complete(&messages, &tools)
        .await
        .expect("completion should succeed");

    assert!(
        !response.tool_calls.is_empty(),
        "model should call spawn_agent, got text: {:?}",
        response.text
    );

    let tool_call = &response.tool_calls[0];
    assert_eq!(tool_call.name, "spawn_agent");
    let args = &tool_call.arguments;
    assert!(args.get("task").is_some(), "must include task parameter");

    // Validate that null optional arrays are handled gracefully
    // (this exercises the same code path as string_array_param)
    let allow = args.get("allow_tools");
    let deny = args.get("deny_tools");

    // Model may send null, [], or omit entirely — all must be acceptable
    if let Some(val) = allow {
        assert!(
            val.is_null() || val.is_array(),
            "allow_tools must be null or array, got: {val}"
        );
    }
    if let Some(val) = deny {
        assert!(
            val.is_null() || val.is_array(),
            "deny_tools must be null or array, got: {val}"
        );
    }
}

/// Multi-turn tool use: model calls tool, receives result, responds.
#[tokio::test]
#[ignore]
async fn multi_turn_tool_use() {
    let p = make_provider(TOOL_MODEL);
    let tools = vec![weather_tool()];

    // Step 1: model should call the tool
    let messages = vec![ChatMessage::user(
        "What's the weather in London? You must use the get_weather tool.",
    )];
    let response = p
        .complete(&messages, &tools)
        .await
        .expect("first turn should succeed");

    assert!(
        !response.tool_calls.is_empty(),
        "should call get_weather, got text: {:?}",
        response.text
    );
    let tc = &response.tool_calls[0];

    // Step 2: provide tool result, model should produce a text response
    let messages = vec![
        ChatMessage::user("What's the weather in London? You must use the get_weather tool."),
        ChatMessage::assistant_with_tools(response.text.clone(), vec![ToolCall {
            id: tc.id.clone(),
            name: tc.name.clone(),
            arguments: tc.arguments.clone(),
            argument_diagnostic: tc.argument_diagnostic.clone(),
            metadata: tc.metadata.clone(),
        }]),
        ChatMessage::tool(&tc.id, r#"{"temperature": 15, "condition": "cloudy"}"#),
    ];

    let final_response = p
        .complete(&messages, &tools)
        .await
        .expect("second turn should succeed");

    let text = final_response.text.expect("should have text response");
    assert!(!text.is_empty(), "final response should not be empty");
}

/// Flash may return separate reasoning or call a tool directly. Replay any
/// reasoning it provides, as the streaming agent loop does, on the next turn.
#[tokio::test]
#[ignore]
async fn flash_streaming_multi_turn_tool_use() {
    let p = make_provider(TEST_MODEL);
    let tools = vec![weather_tool()];
    let prompt =
        "What is the weather in London? Call get_weather exactly once, then report the result.";
    let mut stream = p.stream_with_tools(vec![ChatMessage::user(prompt)], tools.clone());
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut arguments = String::new();
    let mut call = None;
    let mut saw_done = false;
    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::Delta(delta) => text.push_str(&delta),
            StreamEvent::ReasoningDelta(delta) => reasoning.push_str(&delta),
            StreamEvent::ToolCallStart {
                id,
                name,
                index,
                metadata,
            } => {
                assert_eq!(index, 0, "expected one weather lookup");
                call = Some((id, name, metadata));
            },
            StreamEvent::ToolCallArgumentsDelta { index, delta } => {
                assert_eq!(index, 0);
                arguments.push_str(&delta);
            },
            StreamEvent::Done(_) => {
                saw_done = true;
                break;
            },
            StreamEvent::Error(error) => panic!("Flash first turn failed: {error}"),
            _ => {},
        }
    }
    assert!(saw_done, "first turn must finish");
    let (id, name, metadata) = call.expect("Flash should call get_weather");
    assert_eq!(name, "get_weather");
    let tool_call = ToolCall {
        id: id.clone(),
        name,
        arguments: serde_json::from_str(&arguments).expect("valid tool arguments"),
        argument_diagnostic: None,
        metadata,
    };
    let messages = vec![
        ChatMessage::user(prompt),
        ChatMessage::Assistant {
            content: Some(text),
            tool_calls: vec![tool_call],
            reasoning: (!reasoning.is_empty()).then_some(reasoning),
        },
        ChatMessage::tool(id, r#"{"temperature": 15, "condition": "cloudy"}"#),
    ];
    let mut stream = p.stream_with_tools(messages, tools);
    let mut final_text = String::new();
    let mut saw_done = false;
    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::Delta(delta) => final_text.push_str(&delta),
            StreamEvent::Done(_) => {
                saw_done = true;
                break;
            },
            StreamEvent::Error(error) => panic!("Flash follow-up failed: {error}"),
            _ => {},
        }
    }
    assert!(saw_done, "follow-up must finish");
    assert!(
        final_text.contains("15"),
        "must use the tool result: {final_text}"
    );
}

/// Completion-based tool turns must preserve Flash reasoning just like streams.
#[tokio::test]
#[ignore]
async fn flash_non_streaming_multi_turn_tool_use() {
    let p = make_provider(TEST_MODEL);
    let tools = vec![weather_tool()];
    let mut messages = vec![ChatMessage::user(
        "What is the weather in London? Use get_weather, then report the result.",
    )];
    let response = p
        .complete(&messages, &tools)
        .await
        .expect("Flash tool turn");
    assert!(
        !response.tool_calls.is_empty(),
        "Flash should call the tool"
    );
    messages.push(ChatMessage::from(&response));
    for call in &response.tool_calls {
        assert_eq!(call.name, "get_weather");
        messages.push(ChatMessage::tool(
            &call.id,
            r#"{"temperature":15,"condition":"cloudy"}"#,
        ));
    }
    let answer = p
        .complete(&messages, &tools)
        .await
        .expect("Flash follow-up");
    let text = answer
        .text
        .expect("Flash should answer with the tool result");
    assert!(text.contains("15"), "must use the tool result: {text}");
}

// ── Probe ────────────────────────────────────────────────────────────────────

/// Provider probe must succeed against the live API.
#[tokio::test]
#[ignore]
async fn probe_succeeds() {
    let p = make_provider(TEST_MODEL);
    p.probe()
        .await
        .expect("probe should succeed against live MiniMax API");
}

// ── Streaming edge cases ─────────────────────────────────────────────────────

/// Stream must emit at least one Delta and a terminal Done event.
#[tokio::test]
#[ignore]
async fn stream_emits_delta_and_done() {
    let p = make_provider(TEST_MODEL);
    let messages = vec![ChatMessage::user("Say hello in one word.")];
    let mut stream = p.stream(messages);

    let mut saw_delta = false;
    let mut saw_done = false;

    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::Delta(_) => saw_delta = true,
            StreamEvent::Done(_) => {
                saw_done = true;
                break;
            },
            StreamEvent::Error(err) => panic!("stream error: {err}"),
            _ => {},
        }
    }

    assert!(saw_delta, "stream must emit at least one Delta");
    assert!(saw_done, "stream must emit Done");
}

// ── Model catalog validation ─────────────────────────────────────────────────

/// Probe each model in our catalog and report which ones are alive.
/// Fails if any catalog model is unreachable.
#[tokio::test]
#[ignore]
async fn catalog_models_are_live() {
    let mut alive = Vec::new();
    let mut dead = Vec::new();

    for &model_id in KNOWN_MODELS {
        let p = make_provider(model_id);
        match p.probe().await {
            Ok(()) => alive.push(model_id),
            Err(e) => dead.push((model_id, e.to_string())),
        }
    }

    // Report results
    eprintln!("\n=== MiniMax Model Catalog Health ===");
    eprintln!("Alive ({}):", alive.len());
    for m in &alive {
        eprintln!("  OK {m}");
    }
    if !dead.is_empty() {
        eprintln!("Dead ({}):", dead.len());
        for (m, err) in &dead {
            eprintln!("  DEAD {m}: {err}");
        }
    }
    eprintln!("===================================\n");

    assert!(
        dead.is_empty(),
        "catalog contains unreachable models: {dead:?}"
    );
}

/// Try to discover if MiniMax has added new model patterns we don't know about.
///
/// This checks a set of speculative model names (next version bumps) and
/// reports any that respond successfully, so we can update the catalog.
#[tokio::test]
#[ignore]
async fn detect_new_models() {
    // Speculative names: next major/minor versions and variants
    let candidates = [
        "MiniMax-M3",
        "MiniMax-M3-highspeed",
        "MiniMax-M2.8",
        "MiniMax-M2.8-highspeed",
        "MiniMax-M2.9",
        "MiniMax-M2.9-highspeed",
        "MiniMax-M3.0",
        "MiniMax-M3.0-highspeed",
        "MiniMax-M2.7-turbo",
        "MiniMax-M2.7-pro",
        "MiniMax-M2.7-lite",
    ];

    let known: HashSet<&str> = KNOWN_MODELS.iter().copied().collect();
    let mut discovered = Vec::new();

    for &candidate in &candidates {
        if known.contains(candidate) {
            continue;
        }
        let p = make_provider(candidate);
        if p.probe().await.is_ok() {
            discovered.push(candidate);
        }
    }

    if !discovered.is_empty() {
        eprintln!("\n=== NEW MiniMax Models Discovered ===");
        for m in &discovered {
            eprintln!("  -> {m}");
        }
        eprintln!("Update MINIMAX_MODELS in crates/providers/src/model_catalogs.rs");
        eprintln!("=====================================\n");
    }

    // This is informational — we don't fail on new models, just report them.
    // Uncomment to make this a hard failure:
    // assert!(discovered.is_empty(), "new MiniMax models found: {discovered:?}");
}

/// Check if MiniMax now exposes a /models endpoint.
/// If it does, we should enable `supports_model_discovery: true`.
#[tokio::test]
#[ignore]
async fn check_models_endpoint_availability() {
    let key = api_key();

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{MINIMAX_BASE_URL}/models"))
        .header("Authorization", format!("Bearer {}", key.expose_secret()))
        .send()
        .await
        .expect("HTTP request should succeed");

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    eprintln!("\n=== MiniMax /models endpoint ===");
    eprintln!("Status: {status}");
    if status.is_success() {
        // Parse and show model IDs
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body)
            && let Some(models) = json.get("data").and_then(|d| d.as_array())
        {
            let known: HashSet<&str> = KNOWN_MODELS.iter().copied().collect();
            let api_ids: Vec<&str> = models
                .iter()
                .filter_map(|m| m.get("id").and_then(|id| id.as_str()))
                .collect();

            eprintln!("Models from API ({}):", api_ids.len());
            for id in &api_ids {
                let marker = if known.contains(id) {
                    "OK"
                } else {
                    "NEW ->"
                };
                eprintln!("  {marker} {id}");
            }

            // Check for models in catalog but not in API
            let api_set: HashSet<&str> = api_ids.iter().copied().collect();
            let removed: Vec<&&str> = known.iter().filter(|m| !api_set.contains(**m)).collect();
            if !removed.is_empty() {
                eprintln!("Removed from API: {removed:?}");
            }
        }
        eprintln!("-> Consider enabling supports_model_discovery for minimax!");
    } else {
        eprintln!("Models endpoint still returns {status} (expected for MiniMax)");
        if status.as_u16() != 404 {
            eprintln!("Body: {body}");
        }
    }
    eprintln!("================================\n");

    // Informational only — don't fail the test
}
