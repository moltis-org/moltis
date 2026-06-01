use {
    super::ProviderRegistry,
    moltis_agents::model::ChatMessage,
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

fn capture_one_json_request() -> (String, mpsc::Receiver<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 4096];
        let content_length = loop {
            let read = stream.read(&mut chunk).expect("read request");
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
                    .expect("content length");
            }
        };
        let body_start = buffer
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("body")
            + 4;
        while buffer.len() - body_start < content_length {
            let read = stream.read(&mut chunk).expect("read body");
            buffer.extend_from_slice(&chunk[..read]);
        }
        let body = &buffer[body_start..body_start + content_length];
        tx.send(serde_json::from_slice(body).expect("json body"))
            .expect("send body");
        stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 53\r\n\r\n{\"choices\":[{\"message\":{\"content\":\"ok\"}}],\"usage\":{}}").expect("write response");
    });
    (base_url, rx)
}

#[tokio::test]
async fn initial_openai_compat_registration_applies_provider_rewrite_quirks() {
    let (base_url, body_rx) = capture_one_json_request();
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
        .expect("registered minimax model");
    provider
        .complete(
            &[
                ChatMessage::system("sys"),
                ChatMessage::user_named("hello", "Alice"),
            ],
            &[],
        )
        .await
        .expect("completion succeeds");

    let body = body_rx.recv().expect("captured request body");
    let messages = body["messages"].as_array().expect("messages array");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "user");
    assert!(messages[0].get("name").is_none());
    assert_eq!(
        messages[0]["content"],
        "[System Instructions]\nsys\n[End System Instructions]\n\nhello"
    );
}
