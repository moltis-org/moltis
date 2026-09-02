//! Live integration tests for the Fireworks provider.
//!
//! These tests hit the real Fireworks API and require `FIREWORKS_API_KEY` in
//! the environment. They are `#[ignore]`d by default so `cargo test` skips them.
//!
//! Run with:
//!   cargo test --test fireworks_integration -- --ignored

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{collections::HashSet, time::Duration};

use {
    futures::StreamExt,
    moltis_agents::model::{ChatMessage, CompletionResponse, LlmProvider, StreamEvent, ToolCall},
    moltis_providers::openai::OpenAiProvider,
    secrecy::{ExposeSecret, Secret},
};

const FIREWORKS_BASE_URL: &str = "https://api.fireworks.ai/inference/v1";
const TEST_MODEL: &str = "accounts/fireworks/models/gpt-oss-120b";
const TRANSIENT_RETRY_DELAYS: [u64; 3] = [5, 15, 30];

/// Known Fireworks models we catalog. Keep in sync with `FIREWORKS_MODELS` in
/// `crates/providers/src/model_catalogs.rs`.
const KNOWN_MODELS: &[&str] = &[
    "accounts/fireworks/models/kimi-k2p6",
    "accounts/fireworks/models/gpt-oss-120b",
];

fn api_key() -> Secret<String> {
    let key = std::env::var("FIREWORKS_API_KEY")
        .expect("FIREWORKS_API_KEY must be set for integration tests");
    Secret::new(key)
}

const FIREWORKS_PROVIDER_NAME: &str = "fireworks";

fn make_provider(model: &str) -> OpenAiProvider {
    make_named_provider(model, FIREWORKS_PROVIDER_NAME)
}

/// Build a provider with explicit name, applying the same overrides as
/// `register_openai_compatible_providers` — all three production guards
/// (`config_name == "fireworks"`, `/routers/`, `kimi`) are checked.
fn make_named_provider(model: &str, provider_name: &str) -> OpenAiProvider {
    let mut p = OpenAiProvider::new_with_name(
        api_key(),
        model.to_string(),
        FIREWORKS_BASE_URL.to_string(),
        provider_name.to_string(),
    );

    // Mirror Fireworks Kimi compatibility handling from model_catalogs.rs (issue #810).
    if provider_name == "fireworks" && model.contains("/fireworks/") && model.contains("kimi") {
        p = p.with_strict_tools(false).with_reasoning_content(true);
    }

    p
}

fn is_transient_provider_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("http 429")
        || message.contains("too many requests")
        || message.contains("http 503")
        || message.contains("service unavailable")
        || message.contains("overloaded")
}

async fn wait_before_retry(attempt: usize, operation: &str, error: &str) {
    let delay = TRANSIENT_RETRY_DELAYS[attempt];
    eprintln!("retrying Fireworks {operation} after transient provider error in {delay}s: {error}");
    tokio::time::sleep(Duration::from_secs(delay)).await;
}

async fn complete_with_retries(
    provider: &OpenAiProvider,
    messages: &[ChatMessage],
    tools: &[serde_json::Value],
) -> CompletionResponse {
    for attempt in 0..=TRANSIENT_RETRY_DELAYS.len() {
        match provider.complete(messages, tools).await {
            Ok(response) => return response,
            Err(error)
                if attempt < TRANSIENT_RETRY_DELAYS.len()
                    && is_transient_provider_error(&error.to_string()) =>
            {
                wait_before_retry(attempt, "completion", &error.to_string()).await;
            },
            Err(error) => panic!("Fireworks completion should succeed: {error:#}"),
        }
    }

    unreachable!("bounded retry loop returns or panics")
}

async fn probe_with_retries(provider: &OpenAiProvider) -> anyhow::Result<()> {
    for attempt in 0..=TRANSIENT_RETRY_DELAYS.len() {
        match provider.probe().await {
            Ok(()) => return Ok(()),
            Err(error)
                if attempt < TRANSIENT_RETRY_DELAYS.len()
                    && is_transient_provider_error(&error.to_string()) =>
            {
                wait_before_retry(attempt, "probe", &error.to_string()).await;
            },
            Err(error) => return Err(error),
        }
    }

    unreachable!("bounded retry loop returns or returns an error")
}

async fn stream_with_retries(
    provider: &OpenAiProvider,
    messages: Vec<ChatMessage>,
    tools: Vec<serde_json::Value>,
) -> Vec<StreamEvent> {
    for attempt in 0..=TRANSIENT_RETRY_DELAYS.len() {
        let mut events = Vec::new();
        let mut stream = provider.stream_with_tools(messages.clone(), tools.clone());
        let mut retry_error = None;

        while let Some(event) = stream.next().await {
            match event {
                StreamEvent::Error(error)
                    if attempt < TRANSIENT_RETRY_DELAYS.len()
                        && is_transient_provider_error(&error) =>
                {
                    retry_error = Some(error);
                    break;
                },
                event => events.push(event),
            }
        }

        if let Some(error) = retry_error {
            wait_before_retry(attempt, "stream", &error).await;
            continue;
        }

        return events;
    }

    unreachable!("bounded retry loop returns or panics")
}

#[test]
fn classifies_only_transient_provider_errors_for_retries() {
    assert!(is_transient_provider_error(
        "HTTP 503 Service Unavailable: service overloaded, please try again later"
    ));
    assert!(is_transient_provider_error("HTTP 429 Too Many Requests"));
    assert!(!is_transient_provider_error("HTTP 400 invalid request"));
    assert!(!is_transient_provider_error("HTTP 401 unauthorized"));
    assert!(!is_transient_provider_error("HTTP 404 model not found"));
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

    let response = complete_with_retries(&p, &messages, &[]).await;

    let text = response.text.expect("response must contain text");
    assert!(
        text.to_lowercase().contains(&keyword.to_lowercase()),
        "system prompt was not received by model: response = {text:?}"
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

    let events = stream_with_retries(&p, messages, vec![]).await;
    let mut full_text = String::new();
    let mut saw_done = false;

    for event in events {
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

    let response = complete_with_retries(&p, &messages, &tools).await;

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

    let events = stream_with_retries(&p, messages, tools).await;
    let mut saw_tool_start = false;
    let mut saw_done = false;
    let mut tool_name = String::new();

    for event in events {
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
    let response = complete_with_retries(&p, &messages, &tools).await;

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

    let final_response = complete_with_retries(&p, &messages, &tools).await;

    let text = final_response.text.expect("should have text response");
    assert!(!text.is_empty(), "final response should not be empty");
}

// ── Probe ────────────────────────────────────────────────────────────────────

/// Provider probe must succeed against the live API.
#[tokio::test]
#[ignore]
async fn probe_succeeds() {
    let p = make_provider(TEST_MODEL);
    probe_with_retries(&p)
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
    let events = stream_with_retries(&p, messages, vec![]).await;

    let mut saw_delta = false;
    let mut saw_done = false;

    for event in events {
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
        match probe_with_retries(&p).await {
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
        dead.is_empty(),
        "catalog contains unreachable models: {dead:?}"
    );
}

// ── Fireworks Kimi K2.6 regression coverage (issue #810) ────────────────────

const KIMI_MODEL: &str = "accounts/fireworks/models/kimi-k2p6";

/// Basic completion via Fireworks Kimi K2.6 must succeed.
/// Regression test for issue #810 where strict mode schemas caused 400 errors.
#[tokio::test]
#[ignore]
async fn kimi_router_basic_completion() {
    let p = make_provider(KIMI_MODEL);
    let messages = vec![ChatMessage::user(
        "What is 2+2? Answer with just the number.",
    )];

    let response = complete_with_retries(&p, &messages, &[]).await;

    let text = response.text.expect("should have text response");
    assert!(text.contains('4'), "expected '4' in response: {text:?}");
}

/// Tool calling via Fireworks Kimi K2.6 must not return 400.
/// This was the primary regression in issue #810: strict tool schemas were
/// sent to a backend that doesn't support them.
#[tokio::test]
#[ignore]
async fn kimi_router_tool_call_no_400() {
    let p = make_provider(KIMI_MODEL);
    let tools = vec![weather_tool()];
    let messages = vec![ChatMessage::user(
        "What's the weather in Berlin? You must use the get_weather tool.",
    )];

    let response = complete_with_retries(&p, &messages, &tools).await;

    assert!(
        !response.tool_calls.is_empty(),
        "Kimi router should call tools, got text: {:?}",
        response.text
    );
    assert_eq!(response.tool_calls[0].name, "get_weather");
}

/// Multi-turn tool use with Kimi via Fireworks: exercises reasoning_content
/// serialization that Moonshot requires on assistant tool-call messages.
#[tokio::test]
#[ignore]
async fn kimi_router_multi_turn_tool_use() {
    let p = make_provider(KIMI_MODEL);
    let tools = vec![weather_tool()];

    // Step 1: trigger tool call
    let messages = vec![ChatMessage::user(
        "What's the weather in Tokyo? You must use the get_weather tool.",
    )];
    let response = complete_with_retries(&p, &messages, &tools).await;

    assert!(
        !response.tool_calls.is_empty(),
        "should call get_weather, got text: {:?}",
        response.text
    );
    let tc = &response.tool_calls[0];

    // Step 2: provide result
    let messages = vec![
        ChatMessage::user("What's the weather in Tokyo? You must use the get_weather tool."),
        ChatMessage::assistant_with_tools(response.text.clone(), vec![ToolCall {
            id: tc.id.clone(),
            name: tc.name.clone(),
            arguments: tc.arguments.clone(),
            argument_diagnostic: tc.argument_diagnostic.clone(),
            metadata: tc.metadata.clone(),
        }]),
        ChatMessage::tool(&tc.id, r#"{"temperature": 28, "condition": "sunny"}"#),
    ];

    let mut final_response = complete_with_retries(&p, &messages, &tools).await;

    for attempt in 1..=2 {
        if final_response
            .text
            .as_deref()
            .is_some_and(|text| !text.is_empty())
        {
            break;
        }

        tokio::time::sleep(Duration::from_secs(attempt)).await;
        final_response = complete_with_retries(&p, &messages, &tools).await;
    }

    let text = final_response.text.expect("should have text response");
    assert!(!text.is_empty(), "final response should not be empty");
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
        eprintln!("-> Update FIREWORKS_MODELS in crates/providers/src/model_catalogs.rs");
    }
    eprintln!("=========================================\n");
}
