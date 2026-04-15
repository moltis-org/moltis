use super::*;

#[test]
fn voice_messages_are_marked_with_voice_message_kind() {
    let msg: Message = serde_json::from_value(json!({
        "message_id": 1,
        "date": 1,
        "chat": { "id": 42, "type": "private", "first_name": "Alice" },
        "from": {
            "id": 1001,
            "is_bot": false,
            "first_name": "Alice",
            "username": "alice"
        },
        "voice": {
            "file_id": "voice-file-id",
            "file_unique_id": "voice-unique-id",
            "duration": 1,
            "mime_type": "audio/ogg",
            "file_size": 123
        }
    }))
    .expect("deserialize voice message");

    assert!(matches!(
        message_kind(&msg),
        Some(ChannelMessageKind::Voice)
    ));
}

#[test]
fn extract_document_file_from_message() {
    let msg: Message = serde_json::from_value(json!({
        "message_id": 2,
        "date": 1,
        "chat": { "id": 42, "type": "private", "first_name": "Alice" },
        "from": {
            "id": 1001,
            "is_bot": false,
            "first_name": "Alice",
            "username": "alice"
        },
        "caption": "please review",
        "document": {
            "file_id": "doc-file-id",
            "file_unique_id": "doc-unique-id",
            "file_name": "pinned.html",
            "mime_type": "text/html",
            "file_size": 512
        }
    }))
    .expect("deserialize document message");

    let document = extract_document_file(&msg).expect("document should be extracted");
    assert_eq!(document.file_id, "doc-file-id");
    assert_eq!(document.media_type, "text/html");
    assert_eq!(document.file_name.as_deref(), Some("pinned.html"));
}

#[test]
fn extract_document_file_defaults_media_type_when_missing() {
    let msg: Message = serde_json::from_value(json!({
        "message_id": 3,
        "date": 1,
        "chat": { "id": 42, "type": "private", "first_name": "Alice" },
        "from": {
            "id": 1001,
            "is_bot": false,
            "first_name": "Alice",
            "username": "alice"
        },
        "document": {
            "file_id": "doc-file-id",
            "file_unique_id": "doc-unique-id",
            "file_name": "payload.bin",
            "file_size": 128
        }
    }))
    .expect("deserialize document message");

    let document = extract_document_file(&msg).expect("document should be extracted");
    assert_eq!(document.media_type, "application/octet-stream");
}

#[test]
fn should_inline_markdown_document_types() {
    assert!(should_inline_document_text("text/markdown"));
    assert!(should_inline_document_text("text/x-markdown"));
}

#[test]
fn should_inline_text_xml() {
    assert!(should_inline_document_text("text/xml"));
}

#[test]
fn should_inline_after_normalizing_mime_parameters() {
    // should_inline_document_text expects pre-normalized input;
    // verify the full normalize → check pipeline works.
    assert!(should_inline_document_text(&normalize_media_type(
        "text/plain; charset=utf-8"
    )));
    assert!(should_inline_document_text(&normalize_media_type(
        "application/json; charset=utf-8"
    )));
    assert!(should_inline_document_text(&normalize_media_type(
        "text/html; charset=iso-8859-1"
    )));
}

#[test]
fn normalize_media_type_strips_params() {
    assert_eq!(
        normalize_media_type("text/plain; charset=utf-8"),
        "text/plain"
    );
    assert_eq!(normalize_media_type("TEXT/HTML"), "text/html");
    assert_eq!(normalize_media_type("application/json"), "application/json");
}

#[test]
fn is_supported_document_type_checks() {
    assert!(is_supported_document_type("image/png"));
    assert!(is_supported_document_type("text/plain"));
    assert!(is_supported_document_type("application/json"));
    assert!(is_supported_document_type("application/pdf"));
    assert!(!is_supported_document_type("application/octet-stream"));
}

#[test]
fn extract_text_document_content_utf8_boundary() {
    // 3-byte UTF-8 char: € = [0xE2, 0x82, 0xAC]
    let mut data = vec![b'A'; MAX_INLINE_DOCUMENT_BYTES - 1];
    // Place a 3-byte char straddling the boundary
    data.push(0xE2);
    data.push(0x82);
    data.push(0xAC);
    let result = extract_text_document_content(&data, "text/plain").expect("should produce text");
    // Should not end with the replacement character
    assert!(!result.contains('\u{FFFD}'));
}

#[test]
fn extract_text_document_content_cjk_no_replacement_char() {
    // CJK chars are 3 bytes each. Build a buffer that exceeds the byte
    // limit but stays under the char limit (~21K chars < 24K cap), so the
    // char-limit branch is NOT taken. U+FFFD must still not appear.
    // U+4E00 (一) = [0xE4, 0xB8, 0x80]
    let cjk = [0xE4u8, 0xB8, 0x80];
    let char_count = MAX_INLINE_DOCUMENT_BYTES.div_ceil(3); // enough to exceed byte cap
    assert!(
        char_count < MAX_INLINE_DOCUMENT_CHARS,
        "test requires char count below cap"
    );
    let data: Vec<u8> = cjk.iter().copied().cycle().take(char_count * 3).collect();
    assert!(data.len() > MAX_INLINE_DOCUMENT_BYTES);

    let result = extract_text_document_content(&data, "text/plain").expect("should produce text");
    assert!(
        !result.contains('\u{FFFD}'),
        "U+FFFD found in CJK truncation result"
    );
    assert!(result.contains("[Document content truncated]"));
}

#[tokio::test]
async fn document_html_is_inlined_into_chat_body() {
    use axum::{http::Method, routing::any};

    async fn combined_handler(
        method: Method,
        State(state): State<MockTelegramApi>,
        uri: Uri,
        body: Bytes,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        if method == Method::GET {
            return Bytes::from_static(b"<html><body><h1>Pinned</h1></body></html>")
                .into_response();
        }
        telegram_api_handler(State(state), uri, body)
            .await
            .into_response()
    }

    let recorded_requests = Arc::new(Mutex::new(Vec::<CapturedTelegramRequest>::new()));
    let mock_api = MockTelegramApi {
        requests: Arc::clone(&recorded_requests),
    };
    let app = Router::new()
        .route("/{*path}", any(combined_handler))
        .with_state(mock_api);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("serve mock telegram api");
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let api_url = reqwest::Url::parse(&format!("http://{addr}/")).expect("parse api url");
    let bot = Bot::new("test-token").set_api_url(api_url);

    let accounts: AccountStateMap = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let outbound = Arc::new(TelegramOutbound {
        accounts: Arc::clone(&accounts),
    });
    let sink = Arc::new(MockSink::default());
    let account_id = "test-account";

    {
        let mut map = accounts.write().expect("accounts write lock");
        map.insert(account_id.to_string(), AccountState {
            bot: bot.clone(),
            bot_username: Some("test_bot".into()),
            account_id: account_id.to_string(),
            config: TelegramAccountConfig {
                token: Secret::new("test-token".to_string()),
                dm_policy: DmPolicy::Open,
                ..Default::default()
            },
            outbound: Arc::clone(&outbound),
            cancel: CancellationToken::new(),
            message_log: None,
            event_sink: Some(Arc::clone(&sink) as Arc<dyn ChannelEventSink>),
            otp: Mutex::new(OtpState::new(300)),
        });
    }

    let msg: Message = serde_json::from_value(json!({
        "message_id": 9,
        "date": 1,
        "chat": { "id": 42, "type": "private", "first_name": "Alice" },
        "from": {
            "id": 1001,
            "is_bot": false,
            "first_name": "Alice",
            "username": "alice"
        },
        "caption": "Please review this",
        "document": {
            "file_id": "doc-file-id",
            "file_unique_id": "doc-unique-id",
            "file_name": "pinned.html",
            "mime_type": "text/html",
            "file_size": 512
        }
    }))
    .expect("deserialize document message");

    handle_message_direct(msg, &bot, account_id, &accounts)
        .await
        .expect("handle message");

    assert_eq!(
        sink.dispatch_calls
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "text/html documents should be dispatched as text content"
    );

    {
        let texts = sink.dispatched_texts.lock().expect("lock");
        assert_eq!(texts.len(), 1);
        assert!(texts[0].contains("Please review this"));
        assert!(texts[0].contains("[Document: pinned.html (text/html)]"));
        assert!(texts[0].contains("<h1>Pinned</h1>"));
    }
    {
        let documents = sink.dispatched_documents.lock().expect("lock");
        let files = documents[0].as_ref().expect("document metadata");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].display_name, "pinned.html");
        assert_eq!(files[0].stored_filename, "doc-file-id_pinned.html");
        assert_eq!(files[0].mime_type, "text/html");
    }

    {
        let attachments = sink.dispatched_with_attachments.lock().expect("lock");
        assert!(
            attachments.is_empty(),
            "text/html documents should not be sent as image attachments"
        );
    }

    let _ = shutdown_tx.send(());
    server.await.expect("server join");
}

#[tokio::test]
async fn document_pdf_is_inlined_into_chat_body() {
    use axum::{http::Method, routing::any};

    fn generated_pdf_fixture_bytes() -> Option<Vec<u8>> {
        let dir = tempfile::tempdir().ok()?;
        let text_path = dir.path().join("fixture.txt");
        std::fs::write(&text_path, "Hello from generated PDF fixture\n").ok()?;

        let output = std::process::Command::new("cupsfilter")
            .arg(&text_path)
            .output()
            .ok()?;
        if !output.status.success() || output.stdout.is_empty() {
            return None;
        }
        Some(output.stdout)
    }

    let Some(pdf_fixture) = generated_pdf_fixture_bytes() else {
        eprintln!("skipping PDF fixture test because cupsfilter is unavailable");
        return;
    };

    async fn combined_handler(
        method: Method,
        State(state): State<MockTelegramApi>,
        uri: Uri,
        body: Bytes,
        pdf_fixture: Vec<u8>,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        if method == Method::GET {
            return Bytes::from(pdf_fixture).into_response();
        }
        telegram_api_handler(State(state), uri, body)
            .await
            .into_response()
    }

    let recorded_requests = Arc::new(Mutex::new(Vec::<CapturedTelegramRequest>::new()));
    let mock_api = MockTelegramApi {
        requests: Arc::clone(&recorded_requests),
    };
    let app = Router::new()
        .route(
            "/{*path}",
            any(move |method, state, uri, body| {
                combined_handler(method, state, uri, body, pdf_fixture.clone())
            }),
        )
        .with_state(mock_api);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("serve mock telegram api");
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let api_url = reqwest::Url::parse(&format!("http://{addr}/")).expect("parse api url");
    let bot = Bot::new("test-token").set_api_url(api_url);

    let accounts: AccountStateMap = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let outbound = Arc::new(TelegramOutbound {
        accounts: Arc::clone(&accounts),
    });
    let sink = Arc::new(MockSink::default());
    let account_id = "test-account";

    {
        let mut map = accounts.write().expect("accounts write lock");
        map.insert(account_id.to_string(), AccountState {
            bot: bot.clone(),
            bot_username: Some("test_bot".into()),
            account_id: account_id.to_string(),
            config: TelegramAccountConfig {
                token: Secret::new("test-token".to_string()),
                dm_policy: DmPolicy::Open,
                ..Default::default()
            },
            outbound: Arc::clone(&outbound),
            cancel: CancellationToken::new(),
            message_log: None,
            event_sink: Some(Arc::clone(&sink) as Arc<dyn ChannelEventSink>),
            otp: Mutex::new(OtpState::new(300)),
        });
    }

    let msg: Message = serde_json::from_value(json!({
        "message_id": 10,
        "date": 1,
        "chat": { "id": 42, "type": "private", "first_name": "Alice" },
        "from": {
            "id": 1001,
            "is_bot": false,
            "first_name": "Alice",
            "username": "alice"
        },
        "caption": "Summarize this PDF",
        "document": {
            "file_id": "doc-pdf-file-id",
            "file_unique_id": "doc-pdf-unique-id",
            "file_name": "report.pdf",
            "mime_type": "application/pdf",
            "file_size": 512
        }
    }))
    .expect("deserialize document message");

    handle_message_direct(msg, &bot, account_id, &accounts)
        .await
        .expect("handle message");

    assert_eq!(
        sink.dispatch_calls
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "PDF documents should be dispatched as text content"
    );

    {
        let texts = sink.dispatched_texts.lock().expect("lock");
        assert_eq!(texts.len(), 1);
        assert!(texts[0].contains("Summarize this PDF"));
        assert!(texts[0].contains("[Document: report.pdf (application/pdf)]"));
        assert!(texts[0].contains("Hello from generated PDF fixture"));
    }
    {
        let documents = sink.dispatched_documents.lock().expect("lock");
        let files = documents[0].as_ref().expect("document metadata");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].display_name, "report.pdf");
        assert_eq!(files[0].stored_filename, "doc-pdf-file-id_report.pdf");
        assert_eq!(files[0].mime_type, "application/pdf");
    }

    {
        let attachments = sink.dispatched_with_attachments.lock().expect("lock");
        assert!(
            attachments.is_empty(),
            "PDF documents should not be sent as image attachments"
        );
    }

    let _ = shutdown_tx.send(());
    server.await.expect("server join");
}

#[tokio::test]
async fn document_image_is_dispatched_as_attachment() {
    use axum::{http::Method, routing::any};

    async fn combined_handler(
        method: Method,
        State(state): State<MockTelegramApi>,
        uri: Uri,
        body: Bytes,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        if method == Method::GET {
            return Bytes::from_static(b"fake-png-data").into_response();
        }
        telegram_api_handler(State(state), uri, body)
            .await
            .into_response()
    }

    let recorded_requests = Arc::new(Mutex::new(Vec::<CapturedTelegramRequest>::new()));
    let mock_api = MockTelegramApi {
        requests: Arc::clone(&recorded_requests),
    };
    let app = Router::new()
        .route("/{*path}", any(combined_handler))
        .with_state(mock_api);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("serve mock telegram api");
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let api_url = reqwest::Url::parse(&format!("http://{addr}/")).expect("parse api url");
    let bot = Bot::new("test-token").set_api_url(api_url);

    let accounts: AccountStateMap = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let outbound = Arc::new(TelegramOutbound {
        accounts: Arc::clone(&accounts),
    });
    let sink = Arc::new(MockSink::default());
    let account_id = "test-account";

    {
        let mut map = accounts.write().expect("accounts write lock");
        map.insert(account_id.to_string(), AccountState {
            bot: bot.clone(),
            bot_username: Some("test_bot".into()),
            account_id: account_id.to_string(),
            config: TelegramAccountConfig {
                token: Secret::new("test-token".to_string()),
                dm_policy: DmPolicy::Open,
                ..Default::default()
            },
            outbound: Arc::clone(&outbound),
            cancel: CancellationToken::new(),
            message_log: None,
            event_sink: Some(Arc::clone(&sink) as Arc<dyn ChannelEventSink>),
            otp: Mutex::new(OtpState::new(300)),
        });
    }

    let msg: Message = serde_json::from_value(json!({
        "message_id": 10,
        "date": 1,
        "chat": { "id": 42, "type": "private", "first_name": "Alice" },
        "from": {
            "id": 1001,
            "is_bot": false,
            "first_name": "Alice",
            "username": "alice"
        },
        "caption": "What is in this image?",
        "document": {
            "file_id": "doc-image-file-id",
            "file_unique_id": "doc-image-unique-id",
            "file_name": "screenshot.png",
            "mime_type": "image/png",
            "file_size": 512
        }
    }))
    .expect("deserialize document message");

    handle_message_direct(msg, &bot, account_id, &accounts)
        .await
        .expect("handle message");

    assert_eq!(
        sink.dispatch_calls
            .load(std::sync::atomic::Ordering::Relaxed),
        0,
        "image documents should dispatch through attachment pathway"
    );

    {
        let attachments = sink.dispatched_with_attachments.lock().expect("lock");
        assert_eq!(attachments.len(), 1);
        assert!(attachments[0].text.contains("What is in this image?"));
        assert!(
            attachments[0]
                .text
                .contains("[Document: screenshot.png (image/png)]")
        );
        assert_eq!(attachments[0].media_types, vec!["image/png".to_string()]);
        assert_eq!(attachments[0].sizes, vec![13]);
    }
    {
        let documents = sink.dispatched_documents.lock().expect("lock");
        let files = documents[0].as_ref().expect("document metadata");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].display_name, "screenshot.png");
        assert_eq!(files[0].stored_filename, "doc-image-file-i_screenshot.png");
        assert_eq!(files[0].mime_type, "image/png");
    }

    let _ = shutdown_tx.send(());
    server.await.expect("server join");
}
