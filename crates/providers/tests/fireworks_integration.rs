//! Live integration tests for the Fireworks provider.
//!
//! These tests hit the real Fireworks API and require `FIREWORKS_API_KEY` in
//! the environment. They are `#[ignore]`d by default so `cargo test` skips them.
//!
//! Run with:
//!   cargo test --test fireworks_integration -- --ignored

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;

use {
    futures::StreamExt,
    moltis_agents::model::{ChatMessage, LlmProvider, StreamEvent, ToolCall},
    moltis_providers::openai::OpenAiProvider,
    secrecy::{ExposeSecret, Secret},
};

const FIREWORKS_BASE_URL: &str = "https://api.fireworks.ai/inference/v1";
const TEST_MODEL: &str = "accounts/fireworks/models/deepseek-v3p2";

/// Known Fireworks models we catalog. Keep in sync with `FIREWORKS_MODELS` in
/// `crates/providers/src/lib.rs`.
const KNOWN_MODELS: &[&str] = &[
    "accounts/fireworks/routers/kimi-k2p5-turbo",
    "accounts/fireworks/models/deepseek-v3p2",
    "accounts/fireworks/models/qwen3-235b-a22b-instruct-2507",
    "accounts/fireworks/models/llama-v3p1-405b-instruct",
    "accounts/fireworks/models/llama-v3p1-70b-instruct",
    "accounts/fireworks/models/qwen3-coder-480b-a35b-instruct",
    "accounts/fireworks/models/kimi-k2-instruct-0905",
];

fn api_key() -> Secret<String> {
    let key = std::env::var("FIREWORKS_API_KEY")
        .expect("FIREWORKS_API_KEY must be set for integration tests");
    Secret::new(key)
}

fn make_provider(model: &str) -> OpenAiProvider {
    OpenAiProvider::new_with_name(
        api_key(),
        model.to_string(),
        FIREWORKS_BASE_URL.to_string(),
        "fireworks".to_string(),
    )
}

/// Tool schema in moltis-internal flat format.
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

// ── System prompt handling ───────────────────────────────────────────────────

/// System prompt must reach the model (Fireworks uses standard role: "system").
#[tokio::test]
#[ignore]
async fn system_prompt_is_received_non_streaming() {
    let p = make_provider(TEST_MODEL);
    let keyword = "DRAGONFRUIT";
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
async fn system_prompt_is_received_streaming() {
    let p = make_provider(TEST_MODEL);
    let keyword = "STARFRUIT";
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

// ── Tool calling ─────────────────────────────────────────────────────────────

/// Model must be able to call a tool via non-streaming completion.
#[tokio::test]
#[ignore]
async fn tool_call_round_trip_non_streaming() {
    let p = make_provider(TEST_MODEL);
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

/// Streaming variant: model calls a tool with proper streaming events.
#[tokio::test]
#[ignore]
async fn tool_call_round_trip_streaming() {
    let p = make_provider(TEST_MODEL);
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

/// Multi-turn tool use: model calls tool, receives result, responds.
#[tokio::test]
#[ignore]
async fn multi_turn_tool_use() {
    let p = make_provider(TEST_MODEL);
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
        .expect("probe should succeed against live Fireworks API");
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

    eprintln!("\n=== Fireworks Model Catalog Health ===");
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
    eprintln!("=====================================\n");

    assert!(
        alive.contains(&TEST_MODEL),
        "{TEST_MODEL} should be reachable"
    );
}

/// Discover new models via the Fireworks /models endpoint and compare with
/// our static catalog.
#[tokio::test]
#[ignore]
async fn detect_new_models_via_api() {
    let key = api_key();

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{FIREWORKS_BASE_URL}/models"))
        .header("Authorization", format!("Bearer {}", key.expose_secret()))
        .send()
        .await
        .expect("HTTP request should succeed");

    let status = resp.status();
    if !status.is_success() {
        eprintln!("\n=== Fireworks /models endpoint ===");
        eprintln!("Status: {status} (may not be supported)");
        eprintln!("=================================\n");
        return;
    }

    let body: serde_json::Value = resp.json().await.expect("valid JSON response");
    let models = body
        .get("data")
        .and_then(|d| d.as_array())
        .expect("/models should have data array");

    let known: HashSet<&str> = KNOWN_MODELS.iter().copied().collect();
    let api_ids: Vec<&str> = models
        .iter()
        .filter_map(|m| m.get("id").and_then(|id| id.as_str()))
        .collect();

    eprintln!("\n=== Fireworks /models API ({} models) ===", api_ids.len());

    // Show catalog models' status
    for &known_id in KNOWN_MODELS {
        let marker = if api_ids.contains(&known_id) {
            "OK"
        } else {
            "MISSING"
        };
        eprintln!("  {marker} {known_id}");
    }

    // Show new models not in catalog (only accounts/fireworks ones)
    let new_models: Vec<&&str> = api_ids
        .iter()
        .filter(|id| id.starts_with("accounts/fireworks/") && !known.contains(**id))
        .collect();
    if !new_models.is_empty() {
        eprintln!("New fireworks-native models ({}):", new_models.len());
        for id in &new_models {
            eprintln!("  NEW -> {id}");
        }
        eprintln!("-> Update FIREWORKS_MODELS in crates/providers/src/lib.rs");
    }
    eprintln!("=========================================\n");
}
