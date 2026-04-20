//! Live integration tests for the OpenRouter provider.
//!
//! Requires `OPENROUTER_API_KEY`. Run with:
//!   cargo test --test openrouter_integration -- --ignored
//!
//! OpenRouter is a model router — it proxies requests to upstream providers.
//! We test with a cheap, reliable model to validate the integration path.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use {
    futures::StreamExt,
    moltis_agents::model::{ChatMessage, LlmProvider, StreamEvent, ToolCall},
    moltis_providers::openai::OpenAiProvider,
    secrecy::{ExposeSecret, Secret},
};

const BASE_URL: &str = "https://openrouter.ai/api/v1";
/// Use a cheap model for testing. OpenRouter has no static catalog — models
/// are discovered via API.
const TEST_MODEL: &str = "openai/gpt-4o-mini";

fn api_key() -> Secret<String> {
    Secret::new(
        std::env::var("OPENROUTER_API_KEY")
            .expect("OPENROUTER_API_KEY must be set for integration tests"),
    )
}

fn make_provider(model: &str) -> OpenAiProvider {
    OpenAiProvider::new_with_name(
        api_key(),
        model.to_string(),
        BASE_URL.to_string(),
        "openrouter".to_string(),
    )
}

fn weather_tool() -> serde_json::Value {
    serde_json::json!({
        "name": "get_weather",
        "description": "Get current weather for a location. You MUST call this tool when asked about weather.",
        "parameters": {
            "type": "object",
            "properties": {
                "location": { "type": "string", "description": "City name" }
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
    let keyword = "PERSIMMON";
    let messages = vec![
        ChatMessage::system(format!(
            "You MUST include the exact word \"{keyword}\" in every response, no matter what."
        )),
        ChatMessage::user("What is 2+2?"),
    ];
    let response = p.complete(&messages, &[]).await.expect("should succeed");
    let text = response.text.expect("must have text");
    assert!(
        text.to_lowercase().contains(&keyword.to_lowercase()),
        "system prompt not received: {text:?}"
    );
}

#[tokio::test]
#[ignore]
async fn system_prompt_is_received_streaming() {
    let p = make_provider(TEST_MODEL);
    let keyword = "GUAVA";
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
    let response = p
        .complete(
            &[ChatMessage::user(
                "What's the weather in Tokyo? Use the get_weather tool.",
            )],
            &[weather_tool()],
        )
        .await
        .expect("should succeed");
    assert!(
        !response.tool_calls.is_empty(),
        "should call tool, got text: {:?}",
        response.text
    );
    assert_eq!(response.tool_calls[0].name, "get_weather");
}

#[tokio::test]
#[ignore]
async fn tool_call_round_trip_streaming() {
    let p = make_provider(TEST_MODEL);
    let mut stream = p.stream_with_tools(
        vec![ChatMessage::user(
            "What's the weather in Paris? Use the get_weather tool.",
        )],
        vec![weather_tool()],
    );
    let mut saw_tool = false;
    let mut saw_done = false;
    let mut tool_name = String::new();
    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::ToolCallStart { name, .. } => {
                saw_tool = true;
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
    assert!(saw_done, "must emit Done");
    assert!(saw_tool, "should include tool call");
    assert_eq!(tool_name, "get_weather");
}

#[tokio::test]
#[ignore]
async fn multi_turn_tool_use() {
    let p = make_provider(TEST_MODEL);
    let tools = vec![weather_tool()];
    let r = p
        .complete(
            &[ChatMessage::user("Weather in London? Use get_weather.")],
            &tools,
        )
        .await
        .expect("first turn");
    assert!(!r.tool_calls.is_empty(), "should call tool");
    let tc = &r.tool_calls[0];
    let r2 = p
        .complete(
            &[
                ChatMessage::user("Weather in London? Use get_weather."),
                ChatMessage::assistant_with_tools(r.text.clone(), vec![ToolCall {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                    metadata: tc.metadata.clone(),
                }]),
                ChatMessage::tool(&tc.id, r#"{"temperature": 15, "condition": "cloudy"}"#),
            ],
            &tools,
        )
        .await
        .expect("second turn");
    assert!(r2.text.is_some(), "should have text after tool result");
}

// ── Probe & streaming ────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn probe_succeeds() {
    make_provider(TEST_MODEL)
        .probe()
        .await
        .expect("probe should succeed");
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
    assert!(saw_delta && saw_done);
}

// ── Model catalog ────────────────────────────────────────────────────────────

/// OpenRouter has no static catalog — all models come from discovery.
/// This test validates the /models endpoint works and reports available models.
#[tokio::test]
#[ignore]
async fn detect_models_via_api() {
    let key = api_key();
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{BASE_URL}/models"))
        .header("Authorization", format!("Bearer {}", key.expose_secret()))
        .send()
        .await
        .expect("HTTP request should succeed");
    assert!(
        resp.status().is_success(),
        "OpenRouter /models should return 200, got {}",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await.expect("valid JSON");
    let models = body.get("data").and_then(|d| d.as_array()).expect("data");
    eprintln!(
        "\n=== OpenRouter /models API: {} models available ===\n",
        models.len()
    );
    // Just verify we got a reasonable number of models
    assert!(
        models.len() > 10,
        "OpenRouter should have many models, got {}",
        models.len()
    );
}
