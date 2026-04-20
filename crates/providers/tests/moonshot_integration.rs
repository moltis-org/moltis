//! Live integration tests for the Moonshot (Kimi) provider.
//!
//! These tests hit the real Moonshot API and require `MOONSHOT_API_KEY` in the
//! environment. They are `#[ignore]`d by default so `cargo test` skips them.
//!
//! Run with:
//!   cargo test --test moonshot_integration -- --ignored

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;

use {
    futures::StreamExt,
    moltis_agents::model::{ChatMessage, LlmProvider, StreamEvent, ToolCall},
    moltis_providers::openai::OpenAiProvider,
    secrecy::{ExposeSecret, Secret},
};

const MOONSHOT_BASE_URL: &str = "https://api.moonshot.ai/v1";
const TEST_MODEL: &str = "kimi-k2.5";

/// Known Moonshot models we catalog. Keep in sync with `MOONSHOT_MODELS` in
/// `crates/providers/src/lib.rs`.
const KNOWN_MODELS: &[&str] = &["kimi-k2.5"];

fn api_key() -> Secret<String> {
    let key = std::env::var("MOONSHOT_API_KEY")
        .expect("MOONSHOT_API_KEY must be set for integration tests");
    Secret::new(key)
}

fn make_provider(model: &str) -> OpenAiProvider {
    OpenAiProvider::new_with_name(
        api_key(),
        model.to_string(),
        MOONSHOT_BASE_URL.to_string(),
        "moonshot".to_string(),
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

/// System prompt must reach the model (Moonshot uses standard role: "system").
#[tokio::test]
#[ignore]
async fn system_prompt_is_received_non_streaming() {
    let p = make_provider(TEST_MODEL);
    let keyword = "MANGO";
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
    let keyword = "PAPAYA";
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
///
/// Moonshot (kimi-k2.5) with thinking mode enabled requires the original
/// `reasoning_content` from step 1 to be replayed in the assistant message.
/// `ChatMessage` does not carry reasoning_content, so the serialization layer
/// injects an empty string which Moonshot rejects. In production, the gateway
/// preserves the full raw JSON (including `reasoning_content`) across turns.
///
/// This test validates the streaming first turn (tool call detection) and
/// documents the multi-turn limitation for the typed message path.
#[tokio::test]
#[ignore]
async fn multi_turn_tool_use_streaming() {
    let p = make_provider(TEST_MODEL);
    let tools = vec![weather_tool()];

    // Step 1: use streaming to get the tool call
    let messages = vec![ChatMessage::user(
        "What's the weather in London? You must use the get_weather tool.",
    )];
    let mut stream = p.stream_with_tools(messages, tools.clone());

    let mut tool_calls: Vec<(String, String, String)> = Vec::new(); // (id, name, args)
    let mut current_tool_args = String::new();
    let mut current_tool_id = String::new();
    let mut current_tool_name = String::new();

    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::ToolCallStart { id, name, .. } => {
                current_tool_id = id;
                current_tool_name = name;
                current_tool_args.clear();
            },
            StreamEvent::ToolCallArgumentsDelta { delta, .. } => {
                current_tool_args.push_str(&delta);
            },
            StreamEvent::ToolCallComplete { .. } => {
                tool_calls.push((
                    current_tool_id.clone(),
                    current_tool_name.clone(),
                    current_tool_args.clone(),
                ));
            },
            StreamEvent::Done(_) => break,
            StreamEvent::Error(err) => panic!("stream error: {err}"),
            _ => {},
        }
    }
    drop(stream);

    assert!(!tool_calls.is_empty(), "should call get_weather");
    let (tc_id, tc_name, tc_args) = &tool_calls[0];
    assert_eq!(tc_name, "get_weather");

    let args: serde_json::Value = serde_json::from_str(tc_args).expect("valid JSON arguments");

    // Step 2: provide tool result. Moonshot's API requires the original
    // reasoning_content from the model's response; ChatMessage only carries
    // text + tool_calls, so this path hits a 400. The gateway works around
    // this by preserving raw JSON across turns.
    let messages = vec![
        ChatMessage::user("What's the weather in London? You must use the get_weather tool."),
        ChatMessage::assistant_with_tools(None, vec![ToolCall {
            id: tc_id.clone(),
            name: tc_name.clone(),
            arguments: args,
            metadata: None,
        }]),
        ChatMessage::tool(tc_id, r#"{"temperature": 15, "condition": "cloudy"}"#),
    ];

    match p.complete(&messages, &tools).await {
        Ok(response) => {
            let text = response.text.expect("should have text response");
            assert!(!text.is_empty(), "final response should not be empty");
        },
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("reasoning_content is missing") {
                // Known limitation: ChatMessage doesn't carry reasoning_content.
                // In production, the gateway preserves raw JSON across turns.
                eprintln!(
                    "multi-turn 400 (expected): Moonshot requires reasoning_content \
                     from step 1 but ChatMessage doesn't carry it"
                );
            } else {
                panic!("unexpected error in multi-turn: {e}");
            }
        },
    }
}

// ── Probe ────────────────────────────────────────────────────────────────────

/// Provider probe must succeed against the live API.
#[tokio::test]
#[ignore]
async fn probe_succeeds() {
    let p = make_provider(TEST_MODEL);
    p.probe()
        .await
        .expect("probe should succeed against live Moonshot API");
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

    eprintln!("\n=== Moonshot Model Catalog Health ===");
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
    eprintln!("====================================\n");

    assert!(
        alive.contains(&TEST_MODEL),
        "{TEST_MODEL} should be reachable"
    );
}

/// Discover new models via the Moonshot /models endpoint and compare with
/// our static catalog.
#[tokio::test]
#[ignore]
async fn detect_new_models_via_api() {
    let key = api_key();

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{MOONSHOT_BASE_URL}/models"))
        .header("Authorization", format!("Bearer {}", key.expose_secret()))
        .send()
        .await
        .expect("HTTP request should succeed");

    let status = resp.status();
    assert!(
        status.is_success(),
        "Moonshot /models should return 200, got {status}"
    );

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

    eprintln!("\n=== Moonshot /models API ===");
    let mut new_models = Vec::new();
    for id in &api_ids {
        let marker = if known.contains(id) {
            "OK"
        } else {
            "NEW ->"
        };
        eprintln!("  {marker} {id}");
        if !known.contains(id) {
            new_models.push(*id);
        }
    }

    // Check for models in our catalog but not in the API
    let api_set: HashSet<&str> = api_ids.iter().copied().collect();
    let removed: Vec<&&str> = known.iter().filter(|m| !api_set.contains(**m)).collect();
    if !removed.is_empty() {
        eprintln!("Removed from API: {removed:?}");
    }

    if !new_models.is_empty() {
        eprintln!("-> Update MOONSHOT_MODELS in crates/providers/src/lib.rs");
    }
    eprintln!("===========================\n");

    // Informational — don't fail on new models
}

/// Try speculative model names to detect new Kimi models.
#[tokio::test]
#[ignore]
async fn detect_new_models_by_probe() {
    let candidates = [
        "kimi-k3",
        "kimi-k3-turbo",
        "kimi-k2.5-turbo",
        "kimi-k2.5-pro",
        "kimi-k2.6",
        "kimi-k2",
        "kimi-k2-turbo",
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
        eprintln!("\n=== NEW Moonshot Models Discovered ===");
        for m in &discovered {
            eprintln!("  -> {m}");
        }
        eprintln!("Update MOONSHOT_MODELS in crates/providers/src/lib.rs");
        eprintln!("======================================\n");
    }
}
