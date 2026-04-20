//! Live integration tests for the OpenAI provider.
//!
//! These tests hit the real OpenAI API and require `OPENAI_API_KEY` in the
//! environment. They are `#[ignore]`d by default so `cargo test` skips them.
//!
//! Run with:
//!   cargo test --test openai_integration -- --ignored

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;

use {
    futures::StreamExt,
    moltis_agents::model::{ChatMessage, LlmProvider, StreamEvent, ToolCall},
    moltis_providers::openai::OpenAiProvider,
    secrecy::{ExposeSecret, Secret},
};

const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const TEST_MODEL: &str = "gpt-5-mini";

/// Known OpenAI models we catalog. Keep in sync with `DEFAULT_OPENAI_MODELS`
/// in `crates/providers/src/openai.rs`.
const KNOWN_MODELS: &[&str] = &["gpt-5.2", "gpt-5.2-chat-latest", "gpt-5-mini"];

fn api_key() -> Secret<String> {
    let key =
        std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set for integration tests");
    Secret::new(key)
}

fn make_provider(model: &str) -> OpenAiProvider {
    OpenAiProvider::new(api_key(), model.to_string(), OPENAI_BASE_URL.to_string())
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
    let keyword = "BLUEBERRY";
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
    let keyword = "RASPBERRY";
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

    eprintln!("\n=== OpenAI Model Catalog Health ===");
    for m in &alive {
        eprintln!("  OK {m}");
    }
    for (m, err) in &dead {
        eprintln!("  DEAD {m}: {err}");
    }
    eprintln!("==================================\n");

    assert!(alive.contains(&TEST_MODEL), "{TEST_MODEL} should be live");
}

#[tokio::test]
#[ignore]
async fn detect_new_models_via_api() {
    let key = api_key();

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{OPENAI_BASE_URL}/models"))
        .header("Authorization", format!("Bearer {}", key.expose_secret()))
        .send()
        .await
        .expect("HTTP request should succeed");

    assert!(
        resp.status().is_success(),
        "OpenAI /models should return 200"
    );

    let body: serde_json::Value = resp.json().await.expect("valid JSON");
    let models = body.get("data").and_then(|d| d.as_array()).expect("data");

    let known: HashSet<&str> = KNOWN_MODELS.iter().copied().collect();
    let gpt_ids: Vec<&str> = models
        .iter()
        .filter_map(|m| m.get("id").and_then(|id| id.as_str()))
        .filter(|id| {
            id.starts_with("gpt-")
                || id.starts_with("o1")
                || id.starts_with("o3")
                || id.starts_with("o4")
        })
        .collect();

    eprintln!("\n=== OpenAI /models API (chat-capable) ===");
    let mut new_count = 0;
    for id in &gpt_ids {
        if known.contains(id) {
            eprintln!("  OK {id}");
        }
    }
    for id in &gpt_ids {
        if !known.contains(id) {
            eprintln!("  NEW -> {id}");
            new_count += 1;
        }
    }
    if new_count > 0 {
        eprintln!("-> {new_count} new chat-capable models found");
    }
    eprintln!("========================================\n");
}
