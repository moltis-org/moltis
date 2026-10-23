use {
    super::{ProviderRegistry, registration::openai_builtin_capabilities},
    crate::openai::ResponsesWebSocketPolicy,
    anyhow::Context as _,
    moltis_agents::model::{ChatMessage, ToolCall},
    moltis_config::schema::{ProviderEntry, ProvidersConfig},
    secrecy::Secret,
    serde_json::Value,
    std::{
        collections::HashMap,
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
    },
};

const FIREWORKS_KIMI_ROUTER: &str = "accounts/fireworks/routers/kimi-k2p5-turbo";

#[test]
fn reasoning_provider_id_round_trips_and_replaces_suffix() -> anyhow::Result<()> {
    use {moltis_agents::model::ReasoningEffort, std::sync::Arc};

    let mut registry = ProviderRegistry::empty();
    registry.register(
        crate::ModelInfo {
            id: "o3".into(),
            provider: "openai".into(),
            display_name: "o3".into(),
            created_at: None,
            recommended: false,
            capabilities: crate::ModelCapabilities::infer("o3"),
        },
        Arc::new(crate::openai::OpenAiProvider::new(
            Secret::new("test-key".into()),
            "o3".into(),
            "https://api.openai.com/v1".into(),
        )),
    );
    let base = registry.get("openai::o3").context("base provider")?;
    for &effort in ReasoningEffort::ALL {
        let id = format!("openai::o3@reasoning-{}", effort.as_str());
        let explicit = registry.get(&id).context("explicit reasoning")?;
        let default = Arc::clone(&base)
            .with_reasoning_effort(effort)
            .context("default reasoning")?;
        for provider in [explicit, default] {
            assert_eq!(provider.id(), id);
            let restored = registry.get(provider.id()).context("persisted provider")?;
            assert_eq!(restored.reasoning_effort(), Some(effort));
            assert_eq!(restored.id(), id);
            assert!(
                registry
                    .fallback_providers_for(provider.id(), provider.name())
                    .is_empty()
            );
            let changed = provider
                .with_reasoning_effort(ReasoningEffort::Low)
                .context("change effort")?;
            assert_eq!(changed.id(), "openai::o3@reasoning-low");
            assert_eq!(changed.reasoning_effort(), Some(ReasoningEffort::Low));
        }
    }
    assert_eq!(base.id(), "openai::o3");
    assert_eq!(base.reasoning_effort(), None);
    Ok(())
}

#[test]
fn openai_default_base_url_enables_responses_websocket() {
    let capabilities = openai_builtin_capabilities("https://api.openai.com/v1");
    assert_eq!(
        capabilities.responses_websocket_policy,
        ResponsesWebSocketPolicy::OpenAiPlatform,
    );
    assert!(capabilities.responses_required_for_reasoning_tools);
}

#[test]
fn openai_explicit_default_base_url_enables_responses_routing() {
    for base_url in [
        " https://api.openai.com/v1/ ",
        "https://API.OPENAI.COM/v1",
        "https://api.openai.com:443/v1",
    ] {
        let capabilities = openai_builtin_capabilities(base_url);
        assert_eq!(
            capabilities.responses_websocket_policy,
            ResponsesWebSocketPolicy::OpenAiPlatform,
            "{base_url}",
        );
        assert!(
            capabilities.responses_required_for_reasoning_tools,
            "{base_url}"
        );
    }
}

#[test]
fn openai_custom_base_url_disables_responses_websocket() {
    for base_url in [
        "https://openai-compatible.example/v1",
        "http://api.openai.com/v1",
        "https://api.openai.com/v1?proxy=true",
    ] {
        let capabilities = openai_builtin_capabilities(base_url);
        assert_eq!(
            capabilities.responses_websocket_policy,
            ResponsesWebSocketPolicy::Unsupported,
            "{base_url}",
        );
        assert!(
            !capabilities.responses_required_for_reasoning_tools,
            "{base_url}"
        );
    }
}

fn capture_one_json_request() -> anyhow::Result<(String, mpsc::Receiver<anyhow::Result<Value>>)> {
    let listener = TcpListener::bind("127.0.0.1:0").context("bind test server")?;
    let base_url = format!("http://{}", listener.local_addr().context("local addr")?);
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<Value> {
            let (mut stream, _) = listener.accept().context("accept request")?;
            let mut buffer = Vec::new();
            let mut chunk = [0_u8; 4096];
            let content_length = loop {
                let read = stream.read(&mut chunk).context("read request")?;
                anyhow::ensure!(
                    read > 0,
                    "connection closed before request headers completed"
                );
                buffer.extend_from_slice(&chunk[..read]);
                let headers = String::from_utf8_lossy(&buffer);
                if let Some((head, _)) = headers.split_once("\r\n\r\n") {
                    break head
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then_some(value.trim())
                        })
                        .and_then(|value| value.parse::<usize>().ok())
                        .context("content length")?;
                }
            };
            let body_start = buffer
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .context("body")?
                + 4;
            while buffer.len() - body_start < content_length {
                let read = stream.read(&mut chunk).context("read body")?;
                anyhow::ensure!(read > 0, "connection closed before request body completed");
                buffer.extend_from_slice(&chunk[..read]);
            }
            let body = &buffer[body_start..body_start + content_length];
            let value = serde_json::from_slice(body).context("json body")?;
            stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 53\r\n\r\n{\"choices\":[{\"message\":{\"content\":\"ok\"}}],\"usage\":{}}").context("write response")?;
            Ok(value)
        })();
        let _ = tx.send(result);
    });
    Ok((base_url, rx))
}

async fn capture_fireworks_kimi_request(
    model: &str,
    strict_tools: Option<bool>,
) -> anyhow::Result<Value> {
    let (base_url, body_rx) = capture_one_json_request()?;
    let mut config = ProvidersConfig {
        offered: vec!["fireworks".into()],
        ..ProvidersConfig::default()
    };
    config.providers.insert("fireworks".into(), ProviderEntry {
        api_key: Some(Secret::new("test-key".into())),
        base_url: Some(base_url),
        models: vec![model.into()],
        strict_tools,
        ..ProviderEntry::default()
    });

    let mut registry = ProviderRegistry::empty();
    registry.register_openai_compatible_providers(&config, &HashMap::new(), &HashMap::new());
    let provider = registry
        .get(&format!("fireworks::{model}"))
        .context("registered Fireworks Kimi model")?;
    provider
        .complete(
            &[
                ChatMessage::user("weather?"),
                ChatMessage::assistant_with_tools(Some("need weather".into()), vec![ToolCall {
                    id: "call_1".into(),
                    name: "get_weather".into(),
                    arguments: serde_json::json!({"location": "Berlin"}),
                    argument_diagnostic: None,
                    metadata: None,
                }]),
                ChatMessage::tool("call_1", r#"{"temperature":20}"#),
            ],
            &[serde_json::json!({
                "name": "get_weather",
                "description": "Get weather",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "location": { "type": "string" }
                    },
                    "required": ["location"]
                }
            })],
        )
        .await
        .context("completion succeeds")?;

    body_rx
        .recv()
        .context("captured request body")?
        .context("test server captured request body")
}

#[tokio::test]
async fn initial_openai_compat_registration_applies_provider_rewrite_quirks() -> anyhow::Result<()>
{
    let (base_url, body_rx) = capture_one_json_request()?;
    let mut config = ProvidersConfig {
        offered: vec!["minimax".into()],
        ..ProvidersConfig::default()
    };
    config.providers.insert("minimax".into(), ProviderEntry {
        api_key: Some(Secret::new("test-key".into())),
        base_url: Some(base_url),
        models: vec!["MiniMax-M2.7".into()],
        ..ProviderEntry::default()
    });

    let mut registry = ProviderRegistry::empty();
    registry.register_openai_compatible_providers(&config, &HashMap::new(), &HashMap::new());
    let provider = registry
        .get("minimax::MiniMax-M2.7")
        .context("registered minimax model")?;
    provider
        .complete(
            &[
                ChatMessage::system("sys"),
                ChatMessage::user_named("hello", "Alice"),
            ],
            &[],
        )
        .await
        .context("completion succeeds")?;

    let body = body_rx
        .recv()
        .context("captured request body")?
        .context("test server captured request body")?;
    let messages = body["messages"].as_array().context("messages array")?;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "user");
    assert!(messages[0].get("name").is_none());
    assert_eq!(
        messages[0]["content"],
        "[System Instructions]\nsys\n[End System Instructions]\n\nhello"
    );
    Ok(())
}

#[tokio::test]
async fn initial_fireworks_kimi_registration_applies_model_scoped_quirks() -> anyhow::Result<()> {
    for model in [FIREWORKS_KIMI_ROUTER, "accounts/fireworks/models/kimi-k3"] {
        let body = capture_fireworks_kimi_request(model, None).await?;

        assert_eq!(body["model"], model);
        assert_eq!(body["tools"][0]["function"]["strict"], false);
        let messages = body["messages"].as_array().context("messages array")?;
        let assistant_tool_message = messages
            .iter()
            .find(|message| message.get("tool_calls").is_some())
            .context("assistant tool-call message")?;
        assert_eq!(assistant_tool_message["reasoning_content"], "need weather");
    }
    Ok(())
}

#[tokio::test]
async fn explicit_strict_tools_overrides_fireworks_kimi_default() -> anyhow::Result<()> {
    for model in [FIREWORKS_KIMI_ROUTER, "accounts/fireworks/models/kimi-k3"] {
        let body = capture_fireworks_kimi_request(model, Some(true)).await?;

        assert_eq!(body["model"], model);
        assert_eq!(body["tools"][0]["function"]["strict"], true);
    }
    Ok(())
}

#[tokio::test]
async fn tsubasa_registration_uses_its_key_and_text_model_limits() -> anyhow::Result<()> {
    let config = ProvidersConfig {
        offered: vec!["tsubasa".into()],
        ..ProvidersConfig::default()
    };
    let unrelated = HashMap::from([("OPENAI_API_KEY".into(), "unrelated-key".into())]);
    let mut missing = ProviderRegistry::empty();
    missing.register_openai_compatible_providers(&config, &unrelated, &HashMap::new());
    assert!(missing.is_empty());

    for model in ["tsubasa-fast", "tsubasa-pro"] {
        let (base_url, body_rx) = capture_one_json_request()?;
        let env = HashMap::from([
            ("TSUBASA_API_KEY".into(), "test-tsubasa-key".into()),
            ("TSUBASA_BASE_URL".into(), base_url),
            ("OPENAI_API_KEY".into(), "unrelated-key".into()),
        ]);
        let mut registry = ProviderRegistry::empty();
        registry.register_openai_compatible_providers(&config, &env, &HashMap::new());
        let provider = registry
            .get(&format!("tsubasa::{model}"))
            .context("Tsubasa model")?;
        assert_eq!(provider.context_window(), 32_768);
        assert!(!provider.supports_tools());
        assert!(!provider.supports_vision());
        provider
            .complete(&[ChatMessage::user_named("Hello", "reader")], &[])
            .await?;
        let body = body_rx.recv().context("request")??;
        assert_eq!(body["model"], model);
        assert_eq!(
            body["messages"],
            serde_json::json!([{"role":"user","content":"Hello"}])
        );
        assert!(body.get("reasoning_effort").is_none());
    }
    Ok(())
}

#[test]
fn tsubasa_static_models_survive_disabled_discovery() -> anyhow::Result<()> {
    let mut config = ProvidersConfig {
        offered: vec!["tsubasa".into()],
        ..ProvidersConfig::default()
    };
    config.providers.insert("tsubasa".into(), ProviderEntry {
        api_key: Some(Secret::new("test-tsubasa-key".into())),
        fetch_models: false,
        alias: Some("my-tsubasa".into()),
        ..ProviderEntry::default()
    });
    let mut registry = ProviderRegistry::empty();
    registry.register_openai_compatible_providers(&config, &HashMap::new(), &HashMap::new());
    for model in ["tsubasa-fast", "tsubasa-pro"] {
        let provider = registry
            .get(&format!("my-tsubasa::{model}"))
            .context("static Tsubasa model")?;
        assert_eq!(provider.context_window(), 32_768);
        assert!(!provider.supports_tools());
        assert!(!provider.supports_vision());
    }
    Ok(())
}

#[test]
fn tsubasa_model_names_do_not_override_custom_provider_capabilities() -> anyhow::Result<()> {
    let mut config = ProvidersConfig::default();
    config
        .providers
        .insert("custom-other".into(), ProviderEntry {
            api_key: Some(Secret::new("test-custom-key".into())),
            base_url: Some("http://127.0.0.1:1/v1".into()),
            models: vec!["tsubasa-fast".into(), "tsubasa-pro".into()],
            fetch_models: false,
            ..ProviderEntry::default()
        });
    let mut registry = ProviderRegistry::empty();
    registry.register_custom_providers(&config, &HashMap::new());
    for model in ["tsubasa-fast", "tsubasa-pro"] {
        let provider = registry
            .get(&format!("custom-other::{model}"))
            .context("custom model")?;
        assert_eq!(provider.context_window(), 200_000);
        assert!(provider.supports_tools());
        assert!(provider.supports_vision());
    }
    Ok(())
}

#[test]
fn tsubasa_discovery_keeps_context_without_enabling_unqualified_capabilities() -> anyhow::Result<()>
{
    let config = ProvidersConfig {
        offered: vec!["tsubasa".into()],
        ..ProvidersConfig::default()
    };
    let env = HashMap::from([("TSUBASA_API_KEY".into(), "test-tsubasa-key".into())]);
    let mut capabilities = crate::ModelCapabilities::infer("custom-model");
    capabilities.context_window = 65_536;
    let prefetched = HashMap::from([("tsubasa".into(), vec![
        crate::DiscoveredModel::new("tsubasa-fast", "Tsubasa Fast").with_capabilities(capabilities),
    ])]);
    let mut registry = ProviderRegistry::empty();
    registry.register_openai_compatible_providers(&config, &env, &prefetched);
    let provider = registry
        .get("tsubasa::tsubasa-fast")
        .context("discovered model")?;
    assert_eq!(provider.context_window(), 65_536);
    assert!(!provider.supports_tools());
    assert!(!provider.supports_vision());
    Ok(())
}

#[tokio::test]
async fn tsubasa_streams_with_its_key_and_reports_auth_errors() -> anyhow::Result<()> {
    use {
        axum::{Json, Router, response::IntoResponse, routing::post},
        futures::StreamExt as _,
        http::{HeaderMap, StatusCode},
        moltis_agents::model::StreamEvent,
    };

    let (requests_tx, mut requests_rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let requests_tx = requests_tx.clone();
            async move {
                let authorization = headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default();
                let authorized = authorization == "Bearer test-tsubasa-key";
                let _ = requests_tx.send(body);
                if !authorized {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({"error":{"message":"invalid key"}})),
                    )
                        .into_response();
                }
                (
                    [("content-type", "text/event-stream")],
                    concat!(
                        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"}}]}\n\n",
                        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":2}}\n\n",
                        "data: [DONE]\n\n",
                    ),
                )
                    .into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base_url = format!("http://{}/v1", listener.local_addr()?);
    let server = tokio::spawn(async move { axum::serve(listener, app).await });

    for model in ["tsubasa-fast", "tsubasa-pro"] {
        for api_key in ["test-tsubasa-key", "invalid-test-key"] {
            let mut config = ProvidersConfig {
                offered: vec!["tsubasa".into()],
                ..ProvidersConfig::default()
            };
            config.providers.insert("tsubasa".into(), ProviderEntry {
                api_key: Some(Secret::new(api_key.into())),
                base_url: Some(base_url.clone()),
                ..ProviderEntry::default()
            });
            let mut registry = ProviderRegistry::empty();
            registry.register_openai_compatible_providers(
                &config,
                &HashMap::new(),
                &HashMap::new(),
            );
            let provider = registry
                .get(&format!("tsubasa::{model}"))
                .context("Tsubasa model")?;
            let events: Vec<_> = provider
                .stream(vec![ChatMessage::user_named("Hello", "reader")])
                .collect()
                .await;
            let body = requests_rx.recv().await.context("streaming request")?;
            assert_eq!(body["model"], model);
            assert_eq!(body["stream"], true);
            assert_eq!(body["stream_options"]["include_usage"], true);
            assert!(body.get("reasoning_effort").is_none());
            assert!(body["messages"][0].get("name").is_none());
            if api_key == "test-tsubasa-key" {
                assert!(
                    events
                        .iter()
                        .any(|event| matches!(event, StreamEvent::Delta(text) if text == "Hello"))
                );
                assert!(events.iter().any(|event| matches!(event, StreamEvent::Done(usage) if usage.input_tokens == 4 && usage.output_tokens == 2)));
                assert!(
                    !events
                        .iter()
                        .any(|event| matches!(event, StreamEvent::Error(_)))
                );
            } else {
                assert!(events.iter().any(
                    |event| matches!(event, StreamEvent::Error(error) if error.contains("401"))
                ));
                assert!(
                    !events
                        .iter()
                        .any(|event| matches!(event, StreamEvent::Done(_)))
                );
                let error = provider
                    .complete(&[ChatMessage::user("Hello")], &[])
                    .await
                    .err()
                    .context("invalid key must fail")?;
                assert!(error.to_string().contains("401"));
                let _ = requests_rx
                    .recv()
                    .await
                    .context("failed completion request")?;
            }
        }
    }
    server.abort();
    Ok(())
}
