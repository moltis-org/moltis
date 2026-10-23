//! Live integration tests for the Z.AI (Zhipu) provider.
//!
//! Requires `Z_API_KEY`. Run with:
//!   cargo test --test zai_integration -- --ignored

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "support/registered_provider.rs"]
mod registered_provider;

use std::{collections::HashSet, sync::Arc, time::Duration};

use {
    futures::StreamExt,
    moltis_agents::model::{ChatMessage, LlmProvider, ReasoningEffort, StreamEvent},
    secrecy::{ExposeSecret, Secret},
};

const BASE_URL: &str = "https://api.z.ai/api/paas/v4";
// Use the current metered Flash model for the behavioral contract. The older
// free pool has independent quota/overload limits and remains in the catalog
// health report rather than defining availability for every behavioral test.
const TEST_MODEL: &str = "glm-5.3-flash";

const KNOWN_MODELS: &[&str] = &[
    "glm-5.3",
    "glm-5.3-flash",
    "glm-5.3-flashx",
    "glm-5.2",
    "glm-5",
    "glm-4.7",
    "glm-4.7-flash",
    "glm-4.7-flashx",
    "glm-4.6",
    "glm-4.6v",
    "glm-4.6v-flash",
    "glm-4.5",
    "glm-4.5-air",
    "glm-4.5-airx",
    "glm-4.5-flash",
    "glm-4.5v",
    "glm-4-32b-0414-128k",
];

fn api_key() -> Secret<String> {
    Secret::new(std::env::var("Z_API_KEY").expect("Z_API_KEY must be set for integration tests"))
}

fn make_provider(model: &str) -> Arc<dyn LlmProvider> {
    registered_provider::registered_provider("zai", model, BASE_URL, api_key())
}

async fn probe_with_retries(model: &str) -> Result<(), String> {
    let mut last_error = String::new();

    for attempt in 1..=3 {
        match make_provider(model).probe().await {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = error.to_string();
                if attempt < 3 {
                    tokio::time::sleep(Duration::from_secs(attempt)).await;
                }
            },
        }
    }

    Err(last_error)
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
    let keyword = "LOQUAT";
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
    let keyword = "MULBERRY";
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
    let mut messages = vec![
        ChatMessage::user("Weather in London? Use get_weather."),
        ChatMessage::from(&r),
    ];
    messages.extend(
        r.tool_calls.iter().map(|call| {
            ChatMessage::tool(&call.id, r#"{"temperature": 15, "condition": "cloudy"}"#)
        }),
    );
    let r2 = p.complete(&messages, &tools).await.expect("second turn");
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

/// Verify an explicitly configured low-effort stream as well as the default.
#[tokio::test]
#[ignore]
async fn current_flash_model_streams_tool_calls() {
    let provider = make_provider(TEST_MODEL)
        .with_reasoning_effort(ReasoningEffort::Low)
        .expect("reasoning provider");
    let mut stream = provider.stream_with_tools(
        vec![ChatMessage::user(
            "What is the weather in London? Use get_weather.",
        )],
        vec![weather_tool()],
    );
    let mut saw_tool = false;
    let mut saw_done = false;
    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::ToolCallStart { name, .. } => {
                assert_eq!(name, "get_weather");
                saw_tool = true;
            },
            StreamEvent::Done(_) => {
                saw_done = true;
                break;
            },
            StreamEvent::Error(error) => panic!("current Flash stream failed: {error}"),
            _ => {},
        }
    }
    assert!(saw_tool && saw_done);
}

#[tokio::test]
#[ignore]
async fn catalog_models_are_live() {
    let mut alive = Vec::new();
    let mut dead = Vec::new();
    for &m in KNOWN_MODELS {
        match probe_with_retries(m).await {
            Ok(()) => alive.push(m),
            Err(e) => dead.push((m, e)),
        }
    }
    eprintln!("\n=== Z.AI Model Catalog Health ===");
    for m in &alive {
        eprintln!("  OK {m}");
    }
    for (m, e) in &dead {
        eprintln!("  DEAD {m}: {e}");
    }
    eprintln!("================================\n");
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
        eprintln!("Z.AI /models returned {}", resp.status());
        return;
    }
    let body: serde_json::Value = resp.json().await.expect("valid JSON");
    let models = body.get("data").and_then(|d| d.as_array()).expect("data");
    let known: HashSet<&str> = KNOWN_MODELS.iter().copied().collect();
    let api_ids: Vec<&str> = models
        .iter()
        .filter_map(|m| m.get("id").and_then(|id| id.as_str()))
        .collect();
    eprintln!("\n=== Z.AI /models API ({} models) ===", api_ids.len());
    for &k in KNOWN_MODELS {
        let marker = if api_ids.contains(&k) {
            "OK"
        } else {
            "MISSING"
        };
        eprintln!("  {marker} {k}");
    }
    let new: Vec<&&str> = api_ids
        .iter()
        .filter(|id| id.starts_with("glm-") && !known.contains(**id))
        .collect();
    if !new.is_empty() {
        eprintln!("New GLM models ({}):", new.len());
        for id in &new {
            eprintln!("  NEW -> {id}");
        }
    }
    eprintln!("===================================\n");
}
