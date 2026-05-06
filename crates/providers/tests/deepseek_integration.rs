//! Live integration tests for the DeepSeek provider.
//!
//! Requires `DEEPSEEK_API_KEY`. Run with:
//!   cargo test --test deepseek_integration -- --ignored

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;

use {
    futures::StreamExt,
    moltis_agents::model::{ChatMessage, LlmProvider, StreamEvent, ToolCall},
    moltis_providers::openai::OpenAiProvider,
    secrecy::{ExposeSecret, Secret},
};

const BASE_URL: &str = "https://api.deepseek.com";
const TEST_MODEL: &str = "deepseek-chat";

const KNOWN_MODELS: &[&str] = &["deepseek-chat", "deepseek-reasoner"];

fn api_key() -> Secret<String> {
    Secret::new(
        std::env::var("DEEPSEEK_API_KEY")
            .expect("DEEPSEEK_API_KEY must be set for integration tests"),
    )
}

fn make_provider(model: &str) -> OpenAiProvider {
    OpenAiProvider::new_with_name(
        api_key(),
        model.to_string(),
        BASE_URL.to_string(),
        "deepseek".to_string(),
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
    let keyword = "RAMBUTAN";
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
    let keyword = "TAMARIND";
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

#[tokio::test]
#[ignore]
async fn catalog_models_are_live() {
    let mut alive = Vec::new();
    let mut dead = Vec::new();
    for &m in KNOWN_MODELS {
        match make_provider(m).probe().await {
            Ok(()) => alive.push(m),
            Err(e) => dead.push((m, e.to_string())),
        }
    }
    eprintln!("\n=== DeepSeek Model Catalog Health ===");
    for m in &alive {
        eprintln!("  OK {m}");
    }
    for (m, e) in &dead {
        eprintln!("  DEAD {m}: {e}");
    }
    eprintln!("====================================\n");
    assert!(alive.contains(&TEST_MODEL), "{TEST_MODEL} should be live");
}

#[tokio::test]
#[ignore]
async fn detect_new_models_via_api() {
    let key = api_key();
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{BASE_URL}/models"))
        .header("Authorization", format!("Bearer {}", key.expose_secret()))
        .send()
        .await
        .expect("HTTP request should succeed");
    if !resp.status().is_success() {
        eprintln!("DeepSeek /models returned {}", resp.status());
        return;
    }
    let body: serde_json::Value = resp.json().await.expect("valid JSON");
    let models = body.get("data").and_then(|d| d.as_array()).expect("data");
    let known: HashSet<&str> = KNOWN_MODELS.iter().copied().collect();
    let api_ids: Vec<&str> = models
        .iter()
        .filter_map(|m| m.get("id").and_then(|id| id.as_str()))
        .collect();
    eprintln!("\n=== DeepSeek /models API ({} models) ===", api_ids.len());
    for &k in KNOWN_MODELS {
        let marker = if api_ids.contains(&k) {
            "OK"
        } else {
            "MISSING"
        };
        eprintln!("  {marker} {k}");
    }
    let new: Vec<&&str> = api_ids.iter().filter(|id| !known.contains(**id)).collect();
    if !new.is_empty() {
        eprintln!("New models ({}):", new.len());
        for id in &new {
            eprintln!("  NEW -> {id}");
        }
    }
    eprintln!("======================================\n");
}
