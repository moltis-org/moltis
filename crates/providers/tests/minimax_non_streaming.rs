//! Exercise the actual completion parser, spawned-agent loop and request serializer.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use {
    async_trait::async_trait,
    axum::{Json, Router, extract::State, routing::post},
    moltis_agents::{
        model::UserContent,
        runner::{AgentLoopLimits, run_agent_loop_with_context_and_limits},
        tool_registry::{AgentTool, ToolRegistry},
    },
    moltis_config::schema::{ProviderEntry, ProvidersConfig},
    moltis_providers::ProviderRegistry,
    secrecy::Secret,
    serde_json::{Value, json},
};

const FINAL_ANSWER: &str = "The weather tool reports that London is currently 15 degrees Celsius and cloudy. This completes the requested weather lookup.";

struct MockState {
    requests: Mutex<Vec<Value>>,
    reasoning: Option<&'static str>,
    content: &'static str,
}

async fn complete(State(state): State<Arc<MockState>>, Json(body): Json<Value>) -> Json<Value> {
    let is_follow_up = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| message["role"] == "tool");
    state.requests.lock().unwrap().push(body);
    let mut message = if is_follow_up {
        json!({"role": "assistant", "content": FINAL_ANSWER})
    } else {
        json!({
            "role": "assistant",
            "content": state.content,
            "tool_calls": [{
                "id": "call_weather",
                "type": "function",
                "function": {"name": "get_weather", "arguments": "{\"location\":\"London\"}"}
            }]
        })
    };
    if let Some(reasoning) = state.reasoning {
        message["reasoning_content"] = reasoning.into();
    }
    Json(json!({
        "choices": [{"message": message}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5}
    }))
}

struct WeatherTool;

#[async_trait]
impl AgentTool for WeatherTool {
    fn name(&self) -> &str {
        "get_weather"
    }

    fn description(&self) -> &str {
        "Get current weather"
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"location": {"type": "string"}}, "required": ["location"]})
    }

    async fn execute(&self, arguments: Value) -> anyhow::Result<Value> {
        assert_eq!(arguments["location"], "London");
        Ok(json!({"temperature": 15, "condition": "cloudy"}))
    }
}

async fn assert_round_trip(
    reasoning: Option<&'static str>,
    content: &'static str,
    expected_reasoning: Option<&str>,
) {
    let state = Arc::new(MockState {
        requests: Mutex::new(Vec::new()),
        reasoning,
        content,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let router = Router::new()
        .route("/v1/chat/completions", post(complete))
        .with_state(Arc::clone(&state));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut config = ProvidersConfig {
        offered: vec!["minimax".into()],
        ..Default::default()
    };
    config.providers.insert("minimax".into(), ProviderEntry {
        api_key: Some(Secret::new("test-only-key".into())),
        base_url: Some(base_url),
        models: vec!["MiniMax-M3.1-Flash".into()],
        ..Default::default()
    });
    let registry = ProviderRegistry::from_config_with_static_catalogs(
        &config,
        &HashMap::new(),
        HashMap::new(),
    );
    let provider = registry.get("minimax::MiniMax-M3.1-Flash").unwrap();
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(WeatherTool));
    let result = run_agent_loop_with_context_and_limits(
        provider,
        &tools,
        "Use get_weather for weather questions.",
        &UserContent::text("What is the weather in London?"),
        None,
        None,
        None,
        None,
        None,
        AgentLoopLimits {
            max_iterations: Some(2),
        },
    )
    .await
    .unwrap();
    server.abort();
    assert_eq!(result.text, FINAL_ANSWER);
    assert_eq!(result.tool_calls_made, 1);
    let requests = state.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        assert_eq!(request["model"], "MiniMax-M3.1-Flash");
        assert!(
            request.get("stream").is_none(),
            "exercise the non-streaming path"
        );
    }
    let assistant = requests[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["role"] == "assistant")
        .unwrap();
    assert_eq!(assistant["content"], "Checking weather.");
    assert_eq!(assistant["tool_calls"][0]["id"], "call_weather");
    match expected_reasoning {
        Some(reasoning) => assert_eq!(assistant["reasoning_content"], reasoning),
        None => assert!(assistant.get("reasoning_content").is_none()),
    }
}

#[tokio::test]
async fn flash_non_streaming_agent_preserves_separate_reasoning() {
    assert_round_trip(
        Some("  Need live weather.\n"),
        "Checking weather.",
        Some("  Need live weather.\n"),
    )
    .await;
}

#[tokio::test]
async fn flash_non_streaming_agent_omits_absent_reasoning() {
    assert_round_trip(None, "Checking weather.", None).await;
}

#[tokio::test]
async fn flash_non_streaming_agent_omits_empty_reasoning() {
    assert_round_trip(Some(""), "Checking weather.", None).await;
}

#[tokio::test]
async fn flash_non_streaming_agent_preserves_inline_reasoning() {
    assert_round_trip(
        None,
        "<think>Need live weather.</think>Checking weather.",
        Some("Need live weather."),
    )
    .await;
}

#[tokio::test]
async fn flash_non_streaming_agent_prefers_explicit_reasoning() {
    assert_round_trip(
        Some("Explicit reasoning"),
        "<think>Inline reasoning</think>Checking weather.",
        Some("Explicit reasoning"),
    )
    .await;
}
