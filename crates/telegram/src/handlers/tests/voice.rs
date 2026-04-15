use super::*;

#[tokio::test]
async fn voice_not_configured_replies_with_setup_hint_and_skips_dispatch() {
    let recorded_requests = Arc::new(Mutex::new(Vec::<CapturedTelegramRequest>::new()));
    let mock_api = MockTelegramApi {
        requests: Arc::clone(&recorded_requests),
    };
    let app = Router::new()
        .route("/{*path}", post(telegram_api_handler))
        .with_state(mock_api);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
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
    assert!(
        extract_voice_file(&msg).is_some(),
        "message should contain voice media"
    );

    handle_message_direct(msg, &bot, account_id, &accounts)
        .await
        .expect("handle message");

    {
        let requests = recorded_requests.lock().expect("requests lock");
        assert!(
            requests.iter().any(|request| {
                if let CapturedTelegramRequest::SendMessage(body) = request {
                    body.parse_mode.as_deref() == Some("HTML")
                        && is_escaped_reply_to_chat(body, 42, VOICE_REPLY_STT_SETUP_HINT)
                } else {
                    false
                }
            }),
            "expected voice setup hint to be sent, requests={requests:?}"
        );
        assert!(
            requests.iter().any(|request| {
                if let CapturedTelegramRequest::SendChatAction(action) = request {
                    action.chat_id == 42 && action.action == "typing"
                } else {
                    false
                }
            }),
            "expected typing action before reply, requests={requests:?}"
        );
        assert!(
            requests.iter().all(|request| {
                if let CapturedTelegramRequest::Other { method, raw_body } = request {
                    !matches!(
                        method,
                        TelegramApiMethod::SendMessage | TelegramApiMethod::SendChatAction
                    ) || raw_body.is_empty()
                } else {
                    true
                }
            }),
            "unexpected untyped request capture for known method, requests={requests:?}"
        );
    }
    assert_eq!(
        sink.dispatch_calls
            .load(std::sync::atomic::Ordering::Relaxed),
        0,
        "voice message should not be dispatched to chat when STT is unavailable"
    );

    let _ = shutdown_tx.send(());
    server.await.expect("server join");
}

/// Outcome of transcription in a voice-test scenario.
enum VoiceTranscriptionOutcome {
    /// Transcription returns a non-empty transcript.
    Ok(&'static str),
    /// Transcription returns `Ok("")` — the STT heard nothing meaningful.
    Empty,
    /// Transcription returns an error.
    Err,
}

/// Outcome of the Telegram file-download HTTP call in a voice-test
/// scenario.
enum VoiceDownloadOutcome {
    /// Download succeeds with dummy audio bytes.
    Ok,
    /// Download returns HTTP 500.
    Fail,
}

struct VoiceScenarioResult {
    dispatch_calls: usize,
    dispatched_texts: Vec<String>,
    sent_messages: Vec<SendMessageRequest>,
}

/// Run a Telegram voice-message scenario end-to-end through
/// `handle_message_direct` and return everything the assertions below
/// need to verify the dispatch / direct-reply behavior.
///
/// `caption` is attached to the voice JSON as `caption` so it round-trips
/// through `extract_text`. Telegram voice messages support captions per
/// the Bot API.
async fn run_voice_scenario(
    caption: Option<&str>,
    has_event_sink: bool,
    stt_available: bool,
    download: VoiceDownloadOutcome,
    transcription: VoiceTranscriptionOutcome,
) -> VoiceScenarioResult {
    use axum::{
        http::{Method, StatusCode},
        response::IntoResponse,
        routing::any,
    };

    #[derive(Clone)]
    struct CombinedState {
        api: MockTelegramApi,
        download_succeeds: bool,
    }

    async fn combined_handler(
        method: Method,
        State(state): State<CombinedState>,
        uri: Uri,
        body: Bytes,
    ) -> axum::response::Response {
        if method == Method::GET {
            if state.download_succeeds {
                return Bytes::from_static(b"fake-ogg-audio-data").into_response();
            }
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        telegram_api_handler(State(state.api), uri, body)
            .await
            .into_response()
    }

    let recorded_requests = Arc::new(Mutex::new(Vec::<CapturedTelegramRequest>::new()));
    let combined_state = CombinedState {
        api: MockTelegramApi {
            requests: Arc::clone(&recorded_requests),
        },
        download_succeeds: matches!(download, VoiceDownloadOutcome::Ok),
    };
    let app = Router::new()
        .route("/{*path}", any(combined_handler))
        .with_state(combined_state);

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

    let sink = Arc::new(if stt_available {
        let transcription_result = match transcription {
            VoiceTranscriptionOutcome::Ok(text) => Ok(text.to_string()),
            VoiceTranscriptionOutcome::Empty => Ok(String::new()),
            VoiceTranscriptionOutcome::Err => Err(ChannelError::unavailable("mock stt failure")),
        };
        MockSink::with_stt(transcription_result)
    } else {
        MockSink::with_voice_stt(false, None)
    });
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
            event_sink: if has_event_sink {
                Some(Arc::clone(&sink) as Arc<dyn ChannelEventSink>)
            } else {
                None
            },
            otp: Mutex::new(OtpState::new(300)),
        });
    }

    let mut voice_json = json!({
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
    });
    if let Some(caption_text) = caption {
        voice_json
            .as_object_mut()
            .expect("voice json object")
            .insert("caption".to_string(), json!(caption_text));
    }
    let msg: Message = serde_json::from_value(voice_json).expect("deserialize voice message");

    handle_message_direct(msg, &bot, account_id, &accounts)
        .await
        .expect("handle message");

    let dispatch_calls = sink
        .dispatch_calls
        .load(std::sync::atomic::Ordering::Relaxed);
    let dispatched_texts = sink.dispatched_texts.lock().expect("lock").clone();
    let sent_messages: Vec<SendMessageRequest> = recorded_requests
        .lock()
        .expect("lock")
        .iter()
        .filter_map(|req| match req {
            CapturedTelegramRequest::SendMessage(body) => Some(body.clone()),
            _ => None,
        })
        .collect();

    let _ = shutdown_tx.send(());
    server.await.expect("server join");

    VoiceScenarioResult {
        dispatch_calls,
        dispatched_texts,
        sent_messages,
    }
}

/// Regression for https://github.com/moltis-org/moltis/issues/632:
/// when STT returns an empty transcription and there is no caption
/// fallback, the handler must send a direct user-facing reply and
/// **must not** dispatch a placeholder string to the LLM (which would
/// produce a near-empty TTS reply back to the user).
#[tokio::test]
async fn voice_empty_transcription_sends_direct_reply_and_skips_dispatch() {
    let result = run_voice_scenario(
        None,
        true,
        true,
        VoiceDownloadOutcome::Ok,
        VoiceTranscriptionOutcome::Empty,
    )
    .await;

    assert_eq!(
        result.dispatch_calls, 0,
        "empty transcription with no caption must not dispatch to LLM"
    );
    assert!(
        result
            .sent_messages
            .iter()
            .any(|m| m.chat_id == 42 && m.text == VOICE_REPLY_EMPTY_TRANSCRIPTION),
        "expected direct empty-transcription reply, got: {:?}",
        result.sent_messages
    );
}

/// When the voice message has a caption, an empty transcription should
/// fall back to dispatching the caption — the user clearly had text
/// intent so the LLM gets real content, not a placeholder.
#[tokio::test]
async fn voice_empty_transcription_with_caption_dispatches_caption() {
    let result = run_voice_scenario(
        Some("please review the attached audio"),
        true,
        true,
        VoiceDownloadOutcome::Ok,
        VoiceTranscriptionOutcome::Empty,
    )
    .await;

    assert_eq!(
        result.dispatch_calls, 1,
        "caption must be dispatched as the LLM body when transcription is empty"
    );
    assert_eq!(result.dispatched_texts, vec![
        "please review the attached audio".to_string()
    ]);
    assert!(
        result
            .sent_messages
            .iter()
            .all(|m| m.text != VOICE_REPLY_EMPTY_TRANSCRIPTION),
        "direct empty-transcription reply should not be sent when caption is present: {:?}",
        result.sent_messages
    );
}

/// When transcription errors out and there is no caption, the handler
/// must send a direct user-facing reply and must not dispatch a
/// placeholder string to the LLM.
#[tokio::test]
async fn voice_transcription_error_sends_direct_reply_and_skips_dispatch() {
    let result = run_voice_scenario(
        None,
        true,
        true,
        VoiceDownloadOutcome::Ok,
        VoiceTranscriptionOutcome::Err,
    )
    .await;

    assert_eq!(
        result.dispatch_calls, 0,
        "transcription error with no caption must not dispatch to LLM"
    );
    assert!(
        result
            .sent_messages
            .iter()
            .any(|m| m.chat_id == 42 && m.text == VOICE_REPLY_TRANSCRIPTION_FAILED),
        "expected direct transcription-failed reply, got: {:?}",
        result.sent_messages
    );
}

/// When transcription errors out but a caption is present, fall back
/// to dispatching the caption rather than surfacing the error.
#[tokio::test]
async fn voice_transcription_error_with_caption_dispatches_caption() {
    let result = run_voice_scenario(
        Some("summarize this clip"),
        true,
        true,
        VoiceDownloadOutcome::Ok,
        VoiceTranscriptionOutcome::Err,
    )
    .await;

    assert_eq!(
        result.dispatch_calls, 1,
        "caption must be dispatched when transcription errors and a caption is present"
    );
    assert_eq!(result.dispatched_texts, vec![
        "summarize this clip".to_string()
    ]);
    assert!(
        result
            .sent_messages
            .iter()
            .all(|m| m.text != VOICE_REPLY_TRANSCRIPTION_FAILED),
        "direct transcription-failed reply should not be sent when caption is present: {:?}",
        result.sent_messages
    );
}

/// When the file download fails and there is no caption, the handler
/// must send a direct user-facing reply and must not dispatch.
#[tokio::test]
async fn voice_download_failure_sends_direct_reply_and_skips_dispatch() {
    let result = run_voice_scenario(
        None,
        true,
        true,
        VoiceDownloadOutcome::Fail,
        // transcription outcome is irrelevant because we never reach it.
        VoiceTranscriptionOutcome::Ok("unused"),
    )
    .await;

    assert_eq!(
        result.dispatch_calls, 0,
        "download failure with no caption must not dispatch to LLM"
    );
    assert!(
        result
            .sent_messages
            .iter()
            .any(|m| m.chat_id == 42 && m.text == VOICE_REPLY_DOWNLOAD_FAILED),
        "expected direct download-failed reply, got: {:?}",
        result.sent_messages
    );
}

/// When the file download fails but a caption is present, fall back
/// to dispatching the caption.
#[tokio::test]
async fn voice_download_failure_with_caption_dispatches_caption() {
    let result = run_voice_scenario(
        Some("voice note about the design"),
        true,
        true,
        VoiceDownloadOutcome::Fail,
        VoiceTranscriptionOutcome::Ok("unused"),
    )
    .await;

    assert_eq!(
        result.dispatch_calls, 1,
        "caption must be dispatched when voice download fails and a caption is present"
    );
    assert_eq!(result.dispatched_texts, vec![
        "voice note about the design".to_string()
    ]);
    assert!(
        result
            .sent_messages
            .iter()
            .all(|m| m.text != VOICE_REPLY_DOWNLOAD_FAILED),
        "direct download-failed reply should not be sent when caption is present: {:?}",
        result.sent_messages
    );
}

/// Happy path: transcription succeeds and is dispatched as the LLM body.
/// This guards against a refactor regression where the success branch
/// might accidentally stop dispatching.
#[tokio::test]
async fn voice_successful_transcription_dispatches_transcript() {
    let result = run_voice_scenario(
        None,
        true,
        true,
        VoiceDownloadOutcome::Ok,
        VoiceTranscriptionOutcome::Ok("hello world"),
    )
    .await;

    assert_eq!(result.dispatch_calls, 1);
    assert_eq!(result.dispatched_texts, vec!["hello world".to_string()]);
}

/// Happy path with caption: transcript is combined with caption so the
/// LLM gets both the voice content and the user's text framing.
#[tokio::test]
async fn voice_successful_transcription_with_caption_combines_both() {
    let result = run_voice_scenario(
        Some("context: meeting notes"),
        true,
        true,
        VoiceDownloadOutcome::Ok,
        VoiceTranscriptionOutcome::Ok("we decided to ship on friday"),
    )
    .await;

    assert_eq!(result.dispatch_calls, 1);
    assert_eq!(result.dispatched_texts, vec![
        "context: meeting notes\n\n[Voice message]: we decided to ship on friday".to_string()
    ]);
}

#[tokio::test]
async fn voice_stt_unavailable_without_caption_sends_setup_hint_and_skips_dispatch() {
    let result = run_voice_scenario(
        None,
        true,
        false,
        VoiceDownloadOutcome::Ok,
        VoiceTranscriptionOutcome::Ok("unused"),
    )
    .await;

    assert_eq!(result.dispatch_calls, 0);
    assert!(
        result
            .sent_messages
            .iter()
            .any(|m| is_escaped_reply_to_chat(m, 42, VOICE_REPLY_STT_SETUP_HINT)),
        "expected STT setup hint, got: {:?}",
        result.sent_messages
    );
}

#[tokio::test]
async fn voice_stt_unavailable_with_caption_dispatches_caption() {
    let result = run_voice_scenario(
        Some("summarize this anyway"),
        true,
        false,
        VoiceDownloadOutcome::Ok,
        VoiceTranscriptionOutcome::Ok("unused"),
    )
    .await;

    assert_eq!(result.dispatch_calls, 1);
    assert_eq!(result.dispatched_texts, vec![
        "summarize this anyway".to_string()
    ]);
    assert!(
        result
            .sent_messages
            .iter()
            .all(|m| !is_escaped_reply_to_chat(m, 42, VOICE_REPLY_STT_SETUP_HINT)),
        "setup hint should not be sent when caption is present: {:?}",
        result.sent_messages
    );
}

#[tokio::test]
async fn voice_without_event_sink_and_without_caption_sends_unavailable_reply() {
    let result = run_voice_scenario(
        None,
        false,
        false,
        VoiceDownloadOutcome::Ok,
        VoiceTranscriptionOutcome::Ok("unused"),
    )
    .await;

    assert_eq!(result.dispatch_calls, 0);
    assert!(
        result
            .sent_messages
            .iter()
            .any(|m| is_escaped_reply_to_chat(m, 42, VOICE_REPLY_UNAVAILABLE)),
        "expected unavailable reply, got: {:?}",
        result.sent_messages
    );
}

#[tokio::test]
async fn voice_without_event_sink_with_caption_sends_unavailable_reply() {
    let result = run_voice_scenario(
        Some("please use the caption"),
        false,
        false,
        VoiceDownloadOutcome::Ok,
        VoiceTranscriptionOutcome::Ok("unused"),
    )
    .await;

    assert_eq!(result.dispatch_calls, 0);
    assert!(
        result
            .sent_messages
            .iter()
            .any(|m| is_escaped_reply_to_chat(m, 42, VOICE_REPLY_UNAVAILABLE)),
        "expected unavailable reply even with caption, got: {:?}",
        result.sent_messages
    );
}
