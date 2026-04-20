//! Live integration tests for the Anthropic provider.
//!
//! These tests hit the real Anthropic API and require `ANTHROPIC_API_KEY` in
//! the environment. They are `#[ignore]`d by default so `cargo test` skips them.
//!
//! Run with:
//!   cargo test --test anthropic_integration -- --ignored

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;

use {
    futures::StreamExt,
    moltis_agents::model::{ChatMessage, LlmProvider, StreamEvent, ToolCall},
    moltis_providers::anthropic::AnthropicProvider,
    secrecy::{ExposeSecret, Secret},
};

const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
/// Use Haiku for tests — cheapest and fastest.
const TEST_MODEL: &str = "claude-haiku-4-5-20251001";

/// Known Anthropic models we catalog. Keep in sync with `ANTHROPIC_MODELS` in
/// `crates/providers/src/lib.rs`.
const KNOWN_MODELS: &[&str] = &[
    "claude-opus-4-6",
    "claude-sonnet-4-6",
    "claude-opus-4-5-20251101",
    "claude-sonnet-4-5-20250929",
    "claude-haiku-4-5-20251001",
    "claude-opus-4-1-20250805",
    "claude-sonnet-4-20250514",
    "claude-opus-4-20250514",
    "claude-3-7-sonnet-20250219",
    "claude-3-haiku-20240307",
];

fn api_key() -> Secret<String> {
    let key = std::env::var("ANTHROPIC_API_KEY")
        .expect("ANTHROPIC_API_KEY must be set for integration tests");
    Secret::new(key)
}

fn make_provider(model: &str) -> AnthropicProvider {
    AnthropicProvider::new(api_key(), model.to_string(), ANTHROPIC_BASE_URL.to_string())
}

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

// ── System prompt ────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn system_prompt_is_received_non_streaming() {
    let p = make_provider(TEST_MODEL);
    let keyword = "KUMQUAT";
    let messages = vec![
        ChatMessage::system(format!(
            "You MUST include the exact word \"{keyword}\" in every response, no matter what."
        )),
        ChatMessage::user("What is 2+2?"),
    ];

    let response = p
        .complete(&messages, &[])
        .await
        .expect("completion should succeed");

    let text = response.text.expect("response must contain text");
    assert!(
        text.to_lowercase().contains(&keyword.to_lowercase()),
        "system prompt not received: {text:?}"
    );
    assert!(response.usage.input_tokens > 0);
    assert!(response.usage.output_tokens > 0);
}

#[tokio::test]
#[ignore]
async fn system_prompt_is_received_streaming() {
    let p = make_provider(TEST_MODEL);
    let keyword = "LYCHEE";
    let messages = vec![
        ChatMessage::system(format!(
            "You MUST include the exact word \"{keyword}\" in every response, no matter what."
        )),
        ChatMessage::user("What is 3+3?"),
    ];

    let mut stream = p.stream_with_tools(messages, vec![]);
    let mut full_text = String::new();
    let mut saw_done = false;

    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::Delta(chunk) => full_text.push_str(&chunk),
            StreamEvent::Done(_) => {
                saw_done = true;
                break;
            },
            StreamEvent::Error(err) => panic!("stream error: {err}"),
            _ => {},
        }
    }

    assert!(saw_done, "stream must emit Done");
    assert!(
        full_text.to_lowercase().contains(&keyword.to_lowercase()),
        "system prompt not received: {full_text:?}"
    );
}

// ── Tool calling ─────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn tool_call_round_trip_non_streaming() {
    let p = make_provider(TEST_MODEL);
    let messages = vec![ChatMessage::user(
        "What's the weather in Tokyo? Use the get_weather tool.",
    )];

    let response = p
        .complete(&messages, &[weather_tool()])
        .await
        .expect("completion should succeed");

    assert!(
        !response.tool_calls.is_empty(),
        "should call get_weather, got text: {:?}",
        response.text
    );
    assert_eq!(response.tool_calls[0].name, "get_weather");
}

#[tokio::test]
#[ignore]
async fn tool_call_round_trip_streaming() {
    let p = make_provider(TEST_MODEL);
    let messages = vec![ChatMessage::user(
        "What's the weather in Paris? Use the get_weather tool.",
    )];

    let mut stream = p.stream_with_tools(messages, vec![weather_tool()]);
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

    assert!(saw_done, "stream must emit Done");
    assert!(saw_tool_start, "should include a tool call");
    assert_eq!(tool_name, "get_weather");
}

#[tokio::test]
#[ignore]
async fn multi_turn_tool_use() {
    let p = make_provider(TEST_MODEL);
    let tools = vec![weather_tool()];

    let messages = vec![ChatMessage::user(
        "What's the weather in London? Use get_weather.",
    )];
    let response = p.complete(&messages, &tools).await.expect("first turn");
    assert!(!response.tool_calls.is_empty(), "should call tool");
    let tc = &response.tool_calls[0];

    let messages = vec![
        ChatMessage::user("What's the weather in London? Use get_weather."),
        ChatMessage::assistant_with_tools(response.text.clone(), vec![ToolCall {
            id: tc.id.clone(),
            name: tc.name.clone(),
            arguments: tc.arguments.clone(),
            metadata: tc.metadata.clone(),
        }]),
        ChatMessage::tool(&tc.id, r#"{"temperature": 15, "condition": "cloudy"}"#),
    ];

    let final_response = p.complete(&messages, &tools).await.expect("second turn");
    assert!(
        final_response.text.is_some(),
        "should have text after tool result"
    );
}

// ── Probe & streaming ────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn probe_succeeds() {
    let p = make_provider(TEST_MODEL);
    p.probe().await.expect("probe should succeed");
}

#[tokio::test]
#[ignore]
async fn stream_emits_delta_and_done() {
    let p = make_provider(TEST_MODEL);
    let mut stream = p.stream(vec![ChatMessage::user("Say hello in one word.")]);
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

    assert!(saw_delta, "must emit Delta");
    assert!(saw_done, "must emit Done");
}

// ── Model catalog ────────────────────────────────────────────────────────────

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

    eprintln!("\n=== Anthropic Model Catalog Health ===");
    for m in &alive {
        eprintln!("  OK {m}");
    }
    for (m, err) in &dead {
        eprintln!("  DEAD {m}: {err}");
    }
    eprintln!("=====================================\n");

    assert!(alive.contains(&TEST_MODEL), "{TEST_MODEL} should be live");
}

#[tokio::test]
#[ignore]
async fn detect_new_models_via_api() {
    let key = api_key();

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{ANTHROPIC_BASE_URL}/v1/models"))
        .header("x-api-key", key.expose_secret())
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .expect("HTTP request should succeed");

    let status = resp.status();
    if !status.is_success() {
        eprintln!("Anthropic /v1/models returned {status}");
        return;
    }

    let body: serde_json::Value = resp.json().await.expect("valid JSON");
    let models = body.get("data").and_then(|d| d.as_array()).expect("data");

    let known: HashSet<&str> = KNOWN_MODELS.iter().copied().collect();
    let api_ids: Vec<&str> = models
        .iter()
        .filter_map(|m| m.get("id").and_then(|id| id.as_str()))
        .collect();

    eprintln!("\n=== Anthropic /v1/models API ===");
    for &known_id in KNOWN_MODELS {
        let marker = if api_ids.contains(&known_id) {
            "OK"
        } else {
            "MISSING"
        };
        eprintln!("  {marker} {known_id}");
    }

    let new_models: Vec<&&str> = api_ids.iter().filter(|id| !known.contains(**id)).collect();
    if !new_models.is_empty() {
        eprintln!("New models ({}):", new_models.len());
        for id in &new_models {
            eprintln!("  NEW -> {id}");
        }
        eprintln!("-> Update ANTHROPIC_MODELS in crates/providers/src/lib.rs");
    }
    eprintln!("===============================\n");
}
