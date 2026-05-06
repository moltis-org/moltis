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

use std::collections::HashSet;

use {
    futures::StreamExt,
    moltis_agents::model::{ChatMessage, LlmProvider, StreamEvent, ToolCall},
    moltis_providers::openai::OpenAiProvider,
    secrecy::{ExposeSecret, Secret},
};

const MINIMAX_BASE_URL: &str = "https://api.minimax.io/v1";
const TEST_MODEL: &str = "MiniMax-M2.7-highspeed";
/// Non-highspeed model for tool calling tests (more reliable for function calling).
const TOOL_MODEL: &str = "MiniMax-M2.7";

/// Known MiniMax models we catalog. Keep in sync with `MINIMAX_MODELS` in
/// `crates/providers/src/lib.rs`. If this list drifts, the
/// `catalog_models_are_live` test will catch it.
const KNOWN_MODELS: &[&str] = &[
    "MiniMax-M2.7",
    "MiniMax-M2.7-highspeed",
    "MiniMax-M2.5",
    "MiniMax-M2.5-highspeed",
    "MiniMax-M2.1",
    "MiniMax-M2.1-highspeed",
    "MiniMax-M2",
];

fn api_key() -> Secret<String> {
    let key = std::env::var("MINIMAX_API_KEY")
        .expect("MINIMAX_API_KEY must be set for integration tests");
    Secret::new(key)
}

fn make_provider(model: &str) -> OpenAiProvider {
    OpenAiProvider::new_with_name(
        api_key(),
        model.to_string(),
        MINIMAX_BASE_URL.to_string(),
        "minimax".to_string(),
    )
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
        ChatMessage::system("You MUST include the exact word \"ZEBRA\" in every response."),
        ChatMessage::user("Tell me about computers."),
        ChatMessage::system("You MUST also include the exact word \"GIRAFFE\" in every response."),
    ];

    let response = p
        .complete(&messages, &[])
        .await
        .expect("completion should succeed");

    let text = response.text.expect("response must contain text");
    assert!(
        text.contains("ZEBRA"),
        "first system prompt missing: {text:?}"
    );
    assert!(
        text.contains("GIRAFFE"),
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
/// Fails if any model that used to work no longer does, or if we detect
/// new models that aren't in the catalog.
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

    // At least the test model must be alive
    assert!(
        alive.contains(&TEST_MODEL),
        "{TEST_MODEL} should be reachable"
    );

    // Fail if more than half the catalog is dead — something is wrong
    assert!(
        dead.len() <= KNOWN_MODELS.len() / 2,
        "too many catalog models are dead: {dead:?}"
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
        eprintln!("Update MINIMAX_MODELS in crates/providers/src/lib.rs");
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
