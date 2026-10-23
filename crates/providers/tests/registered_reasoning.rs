//! Verify registered providers send the documented thinking and replay fields.

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "support/registered_provider.rs"]
mod registered_provider;

use {
    axum::{Json, Router, extract::State, routing::post},
    moltis_agents::model::{ChatMessage, ReasoningEffort, ToolCall},
    secrecy::Secret,
    serde_json::{Value, json},
    std::sync::{Arc, Mutex},
};

async fn capture(
    State(requests): State<Arc<Mutex<Vec<Value>>>>,
    Json(body): Json<Value>,
) -> Json<Value> {
    requests.lock().unwrap().push(body);
    Json(json!({"choices": [{"message": {"content": "ok", "reasoning_content": ""}}], "usage": {}}))
}

async fn assert_request(provider_name: &str, model: &str, effort: ReasoningEffort, expected: &str) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new()
        .route("/chat/completions", post(capture))
        .with_state(Arc::clone(&requests));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let provider = registered_provider::registered_provider(
        provider_name,
        model,
        &base_url,
        Secret::new("test-only-key".into()),
    )
    .with_reasoning_effort(effort)
    .expect("reasoning provider");
    let messages = vec![
        ChatMessage::user("What is the weather?"),
        ChatMessage::Assistant {
            content: Some("Earlier answer".into()),
            tool_calls: Vec::new(),
            reasoning: Some("Earlier reasoning".into()),
        },
        ChatMessage::user("What about today?"),
        ChatMessage::Assistant {
            content: Some("Checking weather".into()),
            reasoning: Some("Need current weather".into()),
            tool_calls: vec![ToolCall {
                id: "call_weather".into(),
                name: "get_weather".into(),
                arguments: json!({}),
                argument_diagnostic: None,
                metadata: None,
            }],
        },
        ChatMessage::tool("call_weather", "sunny"),
    ];
    let response = provider.complete(&messages, &[json!({"name":"get_weather", "description":"Get weather", "parameters":{"type":"object","properties":{}}})]).await.unwrap();
    assert_eq!(
        response.reasoning.as_deref(),
        Some(""),
        "preserve explicitly empty provider reasoning"
    );
    server.abort();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let body = &requests[0];
    assert_eq!(body["model"], model);
    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(body["reasoning_effort"], expected);
    assert_eq!(
        body["messages"][1]["reasoning_content"],
        "Earlier reasoning"
    );
    assert_eq!(
        body["messages"][3]["reasoning_content"],
        "Need current weather"
    );
    assert_eq!(body["messages"][3]["content"], "Checking weather");
}

const EFFORTS: [(ReasoningEffort, &str); 6] = [
    (ReasoningEffort::Minimal, "low"),
    (ReasoningEffort::Low, "low"),
    (ReasoningEffort::Medium, "high"),
    (ReasoningEffort::High, "high"),
    (ReasoningEffort::ExtraHigh, "high"),
    (ReasoningEffort::Max, "max"),
];

#[tokio::test]
async fn registered_deepseek_enables_thinking_and_replays_full_history() {
    for model in ["deepseek-flash", "deepseek-v4-flash", "deepseek-v4-pro"] {
        for (effort, expected) in EFFORTS {
            assert_request("deepseek", model, effort, expected).await;
        }
    }
}

#[tokio::test]
async fn registered_zai_uses_supported_efforts_and_replays_reasoning() {
    for name in ["zai", "zai-code"] {
        for model in ["glm-5.3", "glm-5.3-flash", "glm-5.3-flashx", "glm-5.2"] {
            for (effort, expected) in EFFORTS {
                assert_request(name, model, effort, expected).await;
            }
        }
    }
}
