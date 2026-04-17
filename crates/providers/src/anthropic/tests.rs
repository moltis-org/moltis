use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    extract::{Query, Request},
    http::StatusCode,
    routing::{get, post},
};

use super::*;

#[derive(Default, Clone)]
struct CapturedRequest {
    body: Option<serde_json::Value>,
}

async fn start_probe_mock() -> (String, Arc<Mutex<Vec<CapturedRequest>>>) {
    let captured: Arc<Mutex<Vec<CapturedRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = captured.clone();

    let app = Router::new().route(
        "/v1/messages",
        post(move |req: Request| {
            let cap = captured_clone.clone();
            async move {
                let body_bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
                    .await
                    .unwrap_or_default();
                let body: Option<serde_json::Value> = serde_json::from_slice(&body_bytes).ok();
                cap.lock().unwrap().push(CapturedRequest { body });

                axum::response::Response::builder()
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from("{}"))
                    .unwrap()
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (format!("http://{addr}"), captured)
}

async fn start_models_mock(
    responses: Arc<Mutex<HashMap<Option<String>, (StatusCode, serde_json::Value)>>>,
) -> String {
    let app = Router::new().route(
        ANTHROPIC_MODELS_ENDPOINT_PATH,
        get(move |Query(params): Query<HashMap<String, String>>| {
            let responses = responses.clone();
            async move {
                let key = params.get("after_id").cloned();
                let (status, body) = responses
                    .lock()
                    .expect("lock responses")
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| {
                        (
                            StatusCode::NOT_FOUND,
                            serde_json::json!({ "error": "missing test response" }),
                        )
                    });
                (status, Json(body))
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    format!("http://{addr}")
}

#[test]
fn retry_after_ms_from_headers_parses_seconds() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::RETRY_AFTER,
        reqwest::header::HeaderValue::from_static("12"),
    );
    assert_eq!(retry_after_ms_from_headers(&headers), Some(12_000));
}

#[test]
fn retry_after_ms_from_headers_ignores_non_numeric_values() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::RETRY_AFTER,
        reqwest::header::HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT"),
    );
    assert_eq!(retry_after_ms_from_headers(&headers), None);
}

#[test]
fn with_retry_after_marker_appends_retry_hint() {
    let base = "HTTP 429: rate limit exceeded".to_string();
    assert_eq!(
        with_retry_after_marker(base.clone(), Some(3_000)),
        "HTTP 429: rate limit exceeded (retry_after_ms=3000)"
    );
    assert_eq!(
        with_retry_after_marker(base.clone(), None),
        "HTTP 429: rate limit exceeded"
    );
}

#[test]
fn apply_thinking_injects_budget_for_high_effort() {
    let provider = AnthropicProvider {
        context_window_global: std::collections::HashMap::new(),
        context_window_provider: std::collections::HashMap::new(),
        api_key: secrecy::Secret::new("test".into()),
        model: "claude-opus-4-5-20251101".into(),
        base_url: "https://api.anthropic.com".into(),
        client: crate::shared_http_client(),
        alias: None,
        reasoning_effort: Some(moltis_agents::model::ReasoningEffort::High),
        cache_retention: moltis_config::CacheRetention::Short,
    };
    let mut body = serde_json::json!({ "model": "claude-opus-4-5-20251101", "max_tokens": 4096 });
    provider.apply_thinking(&mut body);

    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(body["thinking"]["budget_tokens"], 32768);
    assert!(body["max_tokens"].as_u64().unwrap() > 32768);
}

#[test]
fn apply_thinking_skipped_when_no_effort() {
    let provider = AnthropicProvider::new(
        secrecy::Secret::new("test".into()),
        "claude-opus-4-5-20251101".into(),
        "https://api.anthropic.com".into(),
    );
    let mut body = serde_json::json!({ "model": "test", "max_tokens": 4096 });
    provider.apply_thinking(&mut body);
    assert!(body.get("thinking").is_none());
}

#[test]
fn apply_thinking_low_effort_budget() {
    let provider = AnthropicProvider {
        context_window_global: std::collections::HashMap::new(),
        context_window_provider: std::collections::HashMap::new(),
        api_key: secrecy::Secret::new("test".into()),
        model: "claude-sonnet-4-5-20250929".into(),
        base_url: "https://api.anthropic.com".into(),
        client: crate::shared_http_client(),
        alias: None,
        reasoning_effort: Some(moltis_agents::model::ReasoningEffort::Low),
        cache_retention: moltis_config::CacheRetention::Short,
    };
    let mut body = serde_json::json!({ "model": "test", "max_tokens": 4096 });
    provider.apply_thinking(&mut body);

    assert_eq!(body["thinking"]["budget_tokens"], 4096);
    assert!(body["max_tokens"].as_u64().unwrap() > 4096);
}

#[test]
fn with_reasoning_effort_creates_new_provider() {
    use std::sync::Arc;

    let provider = Arc::new(AnthropicProvider::new(
        secrecy::Secret::new("test-key".into()),
        "claude-opus-4-5-20251101".into(),
        "https://api.anthropic.com".into(),
    ));
    assert!(provider.reasoning_effort().is_none());

    let with_effort = provider
        .with_reasoning_effort(moltis_agents::model::ReasoningEffort::High)
        .expect("anthropic supports reasoning_effort");
    assert_eq!(
        with_effort.reasoning_effort(),
        Some(moltis_agents::model::ReasoningEffort::High)
    );
    assert_eq!(with_effort.id(), "claude-opus-4-5-20251101");
}

#[tokio::test]
async fn probe_request_caps_anthropic_output_to_one_token() {
    let (base_url, captured) = start_probe_mock().await;
    let provider = AnthropicProvider::new(
        secrecy::Secret::new("test-key".into()),
        "claude-sonnet-4-5-20250929".into(),
        base_url,
    );

    provider.probe().await.expect("probe should succeed");

    let reqs = captured.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    let body = reqs[0].body.as_ref().expect("request should have a body");
    assert_eq!(body["max_tokens"], 1);
}

#[tokio::test]
async fn fetch_models_from_api_paginates_deduplicates_and_marks_first_three_once() {
    let mut responses = HashMap::new();
    responses.insert(
        None,
        (
            StatusCode::OK,
            serde_json::json!({
                "data": [
                    {"id": "claude-opus-4-6", "display_name": "Claude Opus 4.6", "type": "model"},
                    {"id": "claude-sonnet-4-6", "display_name": "Claude Sonnet 4.6", "type": "model"},
                    {"id": "claude-haiku-4-5", "display_name": "Claude Haiku 4.5", "type": "model"}
                ],
                "has_more": true,
                "last_id": "cursor-1"
            }),
        ),
    );
    responses.insert(
        Some("cursor-1".to_string()),
        (
            StatusCode::OK,
            serde_json::json!({
                "data": [
                    {"id": "claude-haiku-4-5", "display_name": "Claude Haiku 4.5", "type": "model"},
                    {"id": "claude-3-7-sonnet-20250219", "display_name": "Claude 3.7 Sonnet", "type": "model"}
                ],
                "has_more": false,
                "last_id": "cursor-2"
            }),
        ),
    );
    let base_url = start_models_mock(Arc::new(Mutex::new(responses))).await;

    let models = fetch_models_from_api(secrecy::Secret::new("test-key".into()), base_url)
        .await
        .expect("model discovery should succeed");

    let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(ids, vec![
        "claude-opus-4-6",
        "claude-sonnet-4-6",
        "claude-haiku-4-5",
        "claude-3-7-sonnet-20250219",
    ]);
    assert!(models[0].recommended);
    assert!(models[1].recommended);
    assert!(models[2].recommended);
    assert!(!models[3].recommended);
}

#[tokio::test]
async fn fetch_models_from_api_ignores_non_chat_entries() {
    let mut responses = HashMap::new();
    responses.insert(
        None,
        (
            StatusCode::OK,
            serde_json::json!({
                "data": [
                    {"id": "claude-sonnet-4-6", "display_name": "Claude Sonnet 4.6", "type": "model"},
                    {"id": "claude-embeddings-v1", "display_name": "Claude Embeddings", "type": "model"},
                    {"id": "claude-opus-4-6", "display_name": "Claude Opus 4.6", "type": "model"}
                ],
                "has_more": false
            }),
        ),
    );
    let base_url = start_models_mock(Arc::new(Mutex::new(responses))).await;

    let models = fetch_models_from_api(secrecy::Secret::new("test-key".into()), base_url)
        .await
        .expect("model discovery should succeed");

    let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(ids, vec!["claude-sonnet-4-6", "claude-opus-4-6"]);
}

#[tokio::test]
async fn fetch_models_from_api_errors_on_http_failure() {
    let mut responses = HashMap::new();
    responses.insert(
        None,
        (
            StatusCode::TOO_MANY_REQUESTS,
            serde_json::json!({ "error": { "message": "rate limited" } }),
        ),
    );
    let base_url = start_models_mock(Arc::new(Mutex::new(responses))).await;

    let err = fetch_models_from_api(secrecy::Secret::new("test-key".into()), base_url)
        .await
        .expect_err("HTTP failure should surface as an error");

    assert!(err.to_string().contains("HTTP 429"));
}

#[tokio::test]
async fn fetch_models_from_api_errors_when_no_chat_models_are_returned() {
    let mut responses = HashMap::new();
    responses.insert(
        None,
        (
            StatusCode::OK,
            serde_json::json!({
                "data": [
                    {"id": "claude-embeddings-v1", "display_name": "Claude Embeddings", "type": "model"}
                ],
                "has_more": false
            }),
        ),
    );
    let base_url = start_models_mock(Arc::new(Mutex::new(responses))).await;

    let err = fetch_models_from_api(secrecy::Secret::new("test-key".into()), base_url)
        .await
        .expect_err("empty chat-capable catalog should error");

    assert!(err.to_string().contains("returned no models"));
}

#[test]
fn to_anthropic_messages_merges_all_system_into_top_level() {
    use moltis_agents::model::{ChatMessage, UserContent};

    let messages = vec![
        ChatMessage::system("You are a helpful assistant."),
        ChatMessage::User {
            content: UserContent::Text("hello".into()),
            name: None,
        },
        ChatMessage::system("The current user datetime is 2026-03-24 01:23:45 CET."),
        ChatMessage::User {
            content: UserContent::Text("what time is it?".into()),
            name: None,
        },
    ];

    let (system_value, out) = to_anthropic_messages(&messages, true);

    let blocks = system_value
        .expect("system should be present")
        .as_array()
        .expect("should be array")
        .clone();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(
        blocks[0]["text"],
        "You are a helpful assistant.\n\nThe current user datetime is 2026-03-24 01:23:45 CET."
    );
    assert_eq!(blocks[0]["cache_control"]["type"], "ephemeral");

    assert_eq!(out.len(), 2);
    assert_eq!(out[0]["role"], "user");
    assert_eq!(out[1]["role"], "user");
}

#[test]
fn system_prompt_serializes_as_content_block_array_with_cache_control() {
    let messages = vec![
        ChatMessage::system("You are a coding assistant."),
        ChatMessage::User {
            content: UserContent::Text("hi".into()),
            name: None,
        },
    ];

    let (system_value, _) = to_anthropic_messages(&messages, true);
    let blocks = system_value
        .expect("system should be present")
        .as_array()
        .expect("should be array")
        .clone();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(blocks[0]["text"], "You are a coding assistant.");
    assert_eq!(blocks[0]["cache_control"]["type"], "ephemeral");
}

#[test]
fn last_user_message_gets_cache_control() {
    let messages = vec![
        ChatMessage::system("sys"),
        ChatMessage::User {
            content: UserContent::Text("first".into()),
            name: None,
        },
        ChatMessage::Assistant {
            content: Some("reply".into()),
            tool_calls: vec![],
        },
        ChatMessage::User {
            content: UserContent::Text("second".into()),
            name: None,
        },
    ];

    let (_, out) = to_anthropic_messages(&messages, true);

    assert_eq!(out[0]["content"], "first");

    let last_user = &out[2];
    assert_eq!(last_user["role"], "user");
    let content = last_user["content"].as_array().expect("should be array");
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "second");
    assert_eq!(content[0]["cache_control"]["type"], "ephemeral");
}

#[test]
fn multimodal_user_message_gets_cache_control_on_last_block() {
    let messages = vec![ChatMessage::User {
        content: UserContent::Multimodal(vec![
            ContentPart::Text("describe this".into()),
            ContentPart::Image {
                media_type: "image/png".into(),
                data: "base64data".into(),
            },
        ]),
        name: None,
    }];

    let (_, out) = to_anthropic_messages(&messages, true);
    let content = out[0]["content"].as_array().expect("should be array");
    assert_eq!(content.len(), 2);

    assert!(content[0].get("cache_control").is_none());
    assert_eq!(content[1]["cache_control"]["type"], "ephemeral");
}

#[test]
fn named_multimodal_prefixes_only_first_text_block() {
    let messages = vec![ChatMessage::User {
        content: UserContent::Multimodal(vec![
            ContentPart::Text("first".into()),
            ContentPart::Image {
                media_type: "image/png".into(),
                data: "base64data".into(),
            },
            ContentPart::Text("second".into()),
        ]),
        name: Some("Alice".into()),
    }];

    let (_, out) = to_anthropic_messages(&messages, false);
    let content = out[0]["content"].as_array().expect("should be array");
    assert_eq!(content[0]["text"], "[Alice]: first");
    assert_eq!(content[2]["text"], "second");
}

#[test]
fn named_multimodal_without_text_inserts_prefix_block() {
    let messages = vec![ChatMessage::User {
        content: UserContent::Multimodal(vec![ContentPart::Image {
            media_type: "image/png".into(),
            data: "base64data".into(),
        }]),
        name: Some("Alice".into()),
    }];

    let (_, out) = to_anthropic_messages(&messages, false);
    let content = out[0]["content"].as_array().expect("should be array");
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "[Alice]:");
    assert_eq!(content[1]["type"], "image");
}

#[test]
fn no_system_returns_none() {
    let messages = vec![ChatMessage::User {
        content: UserContent::Text("hello".into()),
        name: None,
    }];
    let (system_value, _) = to_anthropic_messages(&messages, true);
    assert!(system_value.is_none());
}

#[test]
fn caching_disabled_returns_plain_string_system() {
    let messages = vec![ChatMessage::system("You are helpful."), ChatMessage::User {
        content: UserContent::Text("hi".into()),
        name: None,
    }];

    let (system_value, out) = to_anthropic_messages(&messages, false);

    let sys = system_value.expect("system should be present");
    assert!(sys.is_string(), "expected string, got: {sys:?}");
    assert_eq!(sys, "You are helpful.");

    assert_eq!(out[0]["content"], "hi");
}

#[test]
fn cache_retention_none_skips_cache_control() {
    let provider = AnthropicProvider {
        context_window_global: std::collections::HashMap::new(),
        context_window_provider: std::collections::HashMap::new(),
        api_key: secrecy::Secret::new("test".into()),
        model: "claude-sonnet-4-5-20250929".into(),
        base_url: "https://api.anthropic.com".into(),
        client: crate::shared_http_client(),
        alias: None,
        reasoning_effort: None,
        cache_retention: moltis_config::CacheRetention::None,
    };
    assert!(!provider.caching_enabled());
}

#[test]
fn cache_retention_short_enables_caching() {
    let provider = AnthropicProvider::new(
        secrecy::Secret::new("test".into()),
        "claude-sonnet-4-5-20250929".into(),
        "https://api.anthropic.com".into(),
    );
    assert!(provider.caching_enabled());
}

#[test]
fn normalize_display_name_formats_alias_when_missing() {
    assert_eq!(
        normalize_display_name("claude-sonnet-4-6", None),
        "Claude Sonnet 4.6"
    );
    assert_eq!(
        normalize_display_name("claude-sonnet-4-5-20250929", None),
        "Claude Sonnet 4.5 2025-09-29"
    );
}

#[test]
fn parse_models_payload_does_not_mark_recommendations() {
    let payload = serde_json::json!({
        "data": [
            {"id": "claude-opus-4-6", "display_name": "Claude Opus 4.6", "type": "model"},
            {"id": "claude-sonnet-4-6", "display_name": "Claude Sonnet 4.6", "type": "model"},
            {"id": "claude-haiku-4-5", "display_name": "Claude Haiku 4.5", "type": "model"},
            {"id": "claude-3-7-sonnet-20250219", "display_name": "Claude 3.7 Sonnet", "type": "model"}
        ]
    });

    let models = parse_models_payload(&payload);
    assert_eq!(models.len(), 4);
    assert!(!models[0].recommended);
    assert!(!models[1].recommended);
    assert!(!models[2].recommended);
    assert!(!models[3].recommended);
}

#[test]
fn mark_recommended_models_marks_first_three_globally() {
    let mut models = vec![
        crate::DiscoveredModel::new("claude-opus-4-6", "Claude Opus 4.6"),
        crate::DiscoveredModel::new("claude-sonnet-4-6", "Claude Sonnet 4.6"),
        crate::DiscoveredModel::new("claude-haiku-4-5", "Claude Haiku 4.5"),
        crate::DiscoveredModel::new("claude-3-7-sonnet-20250219", "Claude 3.7 Sonnet"),
    ];

    mark_recommended_models(&mut models);

    assert!(models[0].recommended);
    assert!(models[1].recommended);
    assert!(models[2].recommended);
    assert!(!models[3].recommended);
}
