#![allow(clippy::unwrap_used, clippy::expect_used)]
use {
    axum::{
        Json, Router,
        extract::State,
        http::{StatusCode, Uri},
        routing::post,
    },
    moltis_channels::{
        gating::DmPolicy,
        plugin::{ChannelOutbound, ChannelStreamOutbound, StreamEvent},
    },
    secrecy::Secret,
    serde::{Deserialize, Serialize},
    std::{
        collections::HashMap,
        sync::{Arc, Mutex},
        time::Duration,
    },
    teloxide::{ApiError, RequestError},
    tokio::{sync::oneshot, time::Instant},
    tokio_util::sync::CancellationToken,
};

use crate::{
    config::TelegramAccountConfig,
    markdown::TELEGRAM_MAX_MESSAGE_LEN,
    otp::OtpState,
    state::{AccountState, AccountStateMap},
};

use super::{
    TelegramOutbound,
    formatting::telegram_html_to_plain_text,
    retry::{is_message_not_modified_error, retry_after_duration},
    stream::{
        StreamProgressState, format_stream_progress_html, has_reached_stream_min_initial_chars,
        stream_progress_cleanup_html,
    },
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct SendMessageRequest {
    chat_id: i64,
    text: String,
    #[serde(default)]
    parse_mode: Option<String>,
}

#[derive(Debug, Serialize)]
struct TelegramApiResponse {
    ok: bool,
    result: TelegramMessageResult,
}

#[derive(Debug, Serialize)]
struct TelegramMessageResult {
    message_id: i64,
    date: i64,
    chat: TelegramChat,
    text: String,
}

#[derive(Debug, Serialize)]
struct TelegramChat {
    id: i64,
    #[serde(rename = "type")]
    chat_type: String,
}

#[derive(Clone)]
struct MockTelegramApi {
    requests: Arc<Mutex<Vec<SendMessageRequest>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StreamApiRequest {
    Unknown {
        path: String,
    },
    SendMessage {
        text: String,
        disable_notification: bool,
    },
    EditMessage {
        text: String,
    },
    DeleteMessage {
        message_id: i64,
    },
}

#[derive(Clone)]
struct MockTelegramStreamApi {
    requests: Arc<Mutex<Vec<StreamApiRequest>>>,
    fail_delete: bool,
    fail_cleanup_edit: bool,
    fail_edit_text: Option<String>,
}

async fn send_message_handler(
    State(state): State<MockTelegramApi>,
    Json(body): Json<SendMessageRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    state
        .requests
        .lock()
        .expect("lock requests")
        .push(body.clone());

    if body.parse_mode.as_deref() == Some("HTML") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "ok": false,
                "error_code": 400,
                "description": "Bad Request: can't parse entities: unsupported start tag"
            })),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!(TelegramApiResponse {
            ok: true,
            result: TelegramMessageResult {
                message_id: 1,
                date: 0,
                chat: TelegramChat {
                    id: body.chat_id,
                    chat_type: "private".to_string(),
                },
                text: body.text,
            },
        })),
    )
}

async fn stream_lifecycle_handler(
    State(state): State<MockTelegramStreamApi>,
    uri: Uri,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let path = uri.path();
    if path.ends_with("/sendChatAction")
        || path.ends_with("/send_chat_action")
        || path.ends_with("/SendChatAction")
    {
        return (
            StatusCode::OK,
            Json(serde_json::json!({ "ok": true, "result": true })),
        );
    }

    if path.ends_with("/sendMessage")
        || path.ends_with("/send_message")
        || path.ends_with("/SendMessage")
    {
        state
            .requests
            .lock()
            .expect("lock stream requests")
            .push(StreamApiRequest::SendMessage {
                text: body
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                disable_notification: body
                    .get("disable_notification")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            });
        return (
            StatusCode::OK,
            Json(serde_json::json!(TelegramApiResponse {
                ok: true,
                result: TelegramMessageResult {
                    message_id: 10,
                    date: 0,
                    chat: TelegramChat {
                        id: body
                            .get("chat_id")
                            .and_then(serde_json::Value::as_i64)
                            .unwrap_or(42),
                        chat_type: "private".to_string(),
                    },
                    text: body
                        .get("text")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                },
            })),
        );
    }

    if path.ends_with("/editMessageText")
        || path.ends_with("/edit_message_text")
        || path.ends_with("/EditMessageText")
    {
        let text = body
            .get("text")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        state
            .requests
            .lock()
            .expect("lock stream requests")
            .push(StreamApiRequest::EditMessage { text: text.clone() });
        if (state.fail_cleanup_edit && text == stream_progress_cleanup_html())
            || state.fail_edit_text.as_deref() == Some(text.as_str())
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "ok": false,
                    "error_code": 400,
                    "description": "Bad Request: cleanup edit failed"
                })),
            );
        }
        return (
            StatusCode::OK,
            Json(serde_json::json!(TelegramApiResponse {
                ok: true,
                result: TelegramMessageResult {
                    message_id: body
                        .get("message_id")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(10),
                    date: 0,
                    chat: TelegramChat {
                        id: body
                            .get("chat_id")
                            .and_then(serde_json::Value::as_i64)
                            .unwrap_or(42),
                        chat_type: "private".to_string(),
                    },
                    text,
                },
            })),
        );
    }

    if path.ends_with("/deleteMessage")
        || path.ends_with("/delete_message")
        || path.ends_with("/DeleteMessage")
    {
        state.requests.lock().expect("lock stream requests").push(
            StreamApiRequest::DeleteMessage {
                message_id: body
                    .get("message_id")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or_default(),
            },
        );
        if state.fail_delete {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "ok": false,
                    "error_code": 400,
                    "description": "Bad Request: delete failed"
                })),
            );
        }
        return (
            StatusCode::OK,
            Json(serde_json::json!({ "ok": true, "result": true })),
        );
    }

    state
        .requests
        .lock()
        .expect("lock stream requests")
        .push(StreamApiRequest::Unknown {
            path: path.to_string(),
        });
    (
        StatusCode::OK,
        Json(serde_json::json!({ "ok": true, "result": true })),
    )
}

#[tokio::test]
async fn send_location_unknown_account_returns_error() {
    let accounts: AccountStateMap = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let outbound = TelegramOutbound {
        accounts: Arc::clone(&accounts),
    };

    let result = outbound
        .send_location("nonexistent", "12345", 48.8566, 2.3522, Some("Paris"), None)
        .await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("unknown channel account"),
        "should report unknown channel account"
    );
}

#[test]
fn retry_after_duration_extracts_wait() {
    let err = RequestError::RetryAfter(teloxide::types::Seconds::from_seconds(42));
    assert_eq!(retry_after_duration(&err), Some(Duration::from_secs(42)));
}

#[test]
fn retry_after_duration_ignores_other_errors() {
    let err = RequestError::Io(std::io::Error::other("boom").into());
    assert_eq!(retry_after_duration(&err), None);
}

#[test]
fn telegram_html_to_plain_text_strips_tags_and_decodes_entities() {
    let plain =
        telegram_html_to_plain_text("<b>Hello</b> &amp; <i>world</i><br><code>&lt;ok&gt;</code>");

    assert_eq!(plain, "Hello & world\n<ok>");
}

#[test]
fn telegram_html_to_plain_text_decodes_numeric_entities() {
    let plain = telegram_html_to_plain_text("it&#39;s &#x1F642;");

    assert_eq!(plain, "it's \u{1F642}");
}

#[test]
fn telegram_html_to_plain_text_decodes_uppercase_hex_entities() {
    let plain = telegram_html_to_plain_text("smile &#X1F642;");

    assert_eq!(plain, "smile \u{1F642}");
}

#[test]
fn telegram_html_to_plain_text_preserves_non_tag_angle_bracket_text() {
    let plain = telegram_html_to_plain_text("<code>if a < b && c > d</code>");

    assert_eq!(plain, "if a < b && c > d");
}

#[test]
fn telegram_html_to_plain_text_preserves_preformatted_indentation() {
    let plain = telegram_html_to_plain_text("<pre>    indented</pre>");

    assert_eq!(plain, "    indented");
}

#[test]
fn is_message_not_modified_error_detects_variant() {
    let err = RequestError::Api(ApiError::MessageNotModified);
    assert!(is_message_not_modified_error(&err));
}

#[test]
fn is_message_not_modified_error_ignores_other_errors() {
    let err = RequestError::Io(std::io::Error::other("boom").into());
    assert!(!is_message_not_modified_error(&err));
}

#[test]
fn stream_min_initial_chars_uses_character_count() {
    assert!(has_reached_stream_min_initial_chars(
        "hello".chars().count(),
        5
    ));
    let emoji_count = "\u{1F642}\u{1F642}\u{1F642}".chars().count();
    assert!(has_reached_stream_min_initial_chars(emoji_count, 3));
    assert!(!has_reached_stream_min_initial_chars(emoji_count, 4));
}

#[test]
fn stream_progress_html_omits_progress_heading() {
    let html = format_stream_progress_html("working", false);

    assert!(!html.contains("Progress update"));
    assert!(html.contains("working"));
}

#[test]
fn stream_progress_state_flushes_pending_text_after_throttle() {
    let mut state = StreamProgressState::new(3, 3500);
    let now = Instant::now();

    state.push_delta("hel");
    assert!(state.should_send_initial_progress());
    state.mark_progress_sent(now, "initial");
    state.push_delta("lo");

    assert!(!state.should_flush_progress(now, Duration::from_secs(2)));
    assert!(state.should_flush_progress(now + Duration::from_secs(2), Duration::from_secs(2)));
}

#[test]
fn stream_progress_state_suppresses_flush_until_backoff_expires() {
    let mut state = StreamProgressState::new(3, 3500);
    let now = Instant::now();

    state.push_delta("hel");
    state.mark_progress_sent(now, "initial");
    state.push_delta("lo");
    assert!(state.should_flush_progress(now + Duration::from_secs(2), Duration::from_secs(2)));

    state.defer_progress_until(now + Duration::from_secs(10));

    assert!(!state.should_flush_progress(now + Duration::from_secs(5), Duration::from_secs(2)));
    assert!(state.should_flush_progress(now + Duration::from_secs(10), Duration::from_secs(2)));
}

#[test]
fn stream_progress_state_resets_throttle_when_rendered_html_is_unchanged() {
    let mut state = StreamProgressState::new(3, 3500);
    let now = Instant::now();

    state.push_delta("hel");
    state.mark_progress_sent(now, "same");
    state.push_delta("lo");
    assert!(state.should_flush_progress(now + Duration::from_secs(2), Duration::from_secs(2)));

    state.mark_progress_observed(now + Duration::from_secs(2), "same");

    assert!(!state.should_flush_progress(now + Duration::from_secs(3), Duration::from_secs(2)));
    assert!(!state.should_flush_progress(now + Duration::from_secs(4), Duration::from_secs(2)));
}

#[test]
fn stream_progress_state_keeps_recent_tail_when_progress_exceeds_limit() {
    let mut state = StreamProgressState::new(1, 12);

    state.push_delta("old-prefix ");
    state.push_delta("fresh-tail");
    let html = state.current_progress_html();

    assert!(html.contains("Older progress hidden"));
    assert!(html.contains("fresh-tail"));
    assert!(!html.contains("old-prefix"));
}

#[test]
fn stream_progress_tail_escapes_html_and_stays_under_telegram_limit() {
    let mut state = StreamProgressState::new(1, TELEGRAM_MAX_MESSAGE_LEN);

    state.push_delta(&"<".repeat(TELEGRAM_MAX_MESSAGE_LEN));
    let html = state.current_progress_html();

    assert!(html.contains("&lt;"));
    assert!(!html.contains("<".repeat(10).as_str()));
    assert!(html.len() <= TELEGRAM_MAX_MESSAGE_LEN);
}

#[test]
fn stream_progress_cleanup_marker_points_to_final_answer() {
    let html = stream_progress_cleanup_html();

    assert!(!html.contains("Progress update"));
    assert!(html.contains("Final answer follows"));
}

#[tokio::test]
async fn telegram_streaming_does_not_replace_final_delivery() {
    let accounts: AccountStateMap = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let outbound = Arc::new(TelegramOutbound {
        accounts: Arc::clone(&accounts),
    });
    let account_id = "enabled-account";

    {
        let mut map = accounts.write().expect("accounts write lock");
        map.insert(account_id.to_string(), AccountState {
            bot: teloxide::Bot::new("test-token"),
            bot_username: Some("test_bot".to_string()),
            account_id: account_id.to_string(),
            config: TelegramAccountConfig {
                token: Secret::new("test-token".to_string()),
                dm_policy: DmPolicy::Open,
                ..Default::default()
            },
            outbound: Arc::clone(&outbound),
            cancel: CancellationToken::new(),
            message_log: None,
            event_sink: None,
            otp: Mutex::new(OtpState::new(300)),
        });
    }

    assert!(outbound.is_stream_enabled(account_id).await);
    assert!(!outbound.streams_final_replies(account_id).await);
}

#[tokio::test]
async fn cleanup_progress_message_reports_failed_delete_and_failed_marker_edit() {
    let recorded_requests = Arc::new(Mutex::new(Vec::<StreamApiRequest>::new()));
    let mock_api = MockTelegramStreamApi {
        requests: Arc::clone(&recorded_requests),
        fail_delete: true,
        fail_cleanup_edit: true,
        fail_edit_text: None,
    };
    let app = Router::new()
        .route("/{*path}", post(stream_lifecycle_handler))
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

    let api_url = reqwest::Url::parse(&format!("http://{addr}/")).expect("parse api url");
    let bot = teloxide::Bot::new("test-token").set_api_url(api_url);
    let accounts: AccountStateMap = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let outbound = TelegramOutbound { accounts };

    let cleaned_up = outbound
        .cleanup_progress_message(
            &bot,
            "test-account",
            "42",
            teloxide::types::ChatId(42),
            teloxide::types::MessageId(10),
        )
        .await;

    assert!(!cleaned_up);
    {
        let requests = recorded_requests.lock().expect("requests lock");
        assert!(
            requests.iter().any(
                |request| matches!(request, StreamApiRequest::DeleteMessage { message_id: 10 })
            )
        );
        assert!(requests.iter().any(|request| matches!(
            request,
            StreamApiRequest::EditMessage { text } if text == stream_progress_cleanup_html()
        )));
    }

    let _ = shutdown_tx.send(());
    server.await.expect("server join");
}

async fn run_stream_lifecycle(
    notify_on_complete: bool,
    events: Vec<(StreamEvent, Duration)>,
) -> Vec<StreamApiRequest> {
    run_stream_lifecycle_with_edit_failure(notify_on_complete, events, None).await
}

async fn run_stream_lifecycle_with_edit_failure(
    notify_on_complete: bool,
    events: Vec<(StreamEvent, Duration)>,
    fail_edit_text: Option<String>,
) -> Vec<StreamApiRequest> {
    let recorded_requests = Arc::new(Mutex::new(Vec::<StreamApiRequest>::new()));
    let mock_api = MockTelegramStreamApi {
        requests: Arc::clone(&recorded_requests),
        fail_delete: false,
        fail_cleanup_edit: false,
        fail_edit_text,
    };
    let app = Router::new()
        .route("/{*path}", post(stream_lifecycle_handler))
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

    let api_url = reqwest::Url::parse(&format!("http://{addr}/")).expect("parse api url");
    let bot = teloxide::Bot::new("test-token").set_api_url(api_url);

    let accounts: AccountStateMap = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let outbound = Arc::new(TelegramOutbound {
        accounts: Arc::clone(&accounts),
    });
    let account_id = "test-account";

    {
        let mut map = accounts.write().expect("accounts write lock");
        map.insert(account_id.to_string(), AccountState {
            bot: bot.clone(),
            bot_username: Some("test_bot".to_string()),
            account_id: account_id.to_string(),
            config: TelegramAccountConfig {
                token: Secret::new("test-token".to_string()),
                dm_policy: DmPolicy::Open,
                edit_throttle_ms: 250,
                stream_notify_on_complete: notify_on_complete,
                stream_min_initial_chars: 3,
                ..Default::default()
            },
            outbound: Arc::clone(&outbound),
            cancel: CancellationToken::new(),
            message_log: None,
            event_sink: None,
            otp: Mutex::new(OtpState::new(300)),
        });
    }

    let (tx, rx) = tokio::sync::mpsc::channel(16);
    let outbound_for_stream = Arc::clone(&outbound);
    let stream_task = tokio::spawn(async move {
        outbound_for_stream
            .send_stream(account_id, "42", None, rx)
            .await
    });

    for (event, delay) in events {
        tx.send(event).await.expect("send stream event");
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }
    drop(tx);

    stream_task
        .await
        .expect("stream task joins")
        .expect("stream completes");

    let _ = shutdown_tx.send(());
    server.await.expect("server join");

    recorded_requests.lock().expect("requests lock").clone()
}

#[tokio::test]
async fn send_stream_reuses_progress_message_for_final_when_notify_disabled() {
    let requests = run_stream_lifecycle(false, vec![
        (
            StreamEvent::ProgressDelta("hel".to_string()),
            Duration::from_millis(25),
        ),
        (
            StreamEvent::ProgressDelta("lo".to_string()),
            Duration::from_millis(600),
        ),
        (
            StreamEvent::Delta("**done**".to_string()),
            Duration::from_millis(25),
        ),
        (StreamEvent::Done, Duration::ZERO),
    ])
    .await;

    assert_eq!(
        requests
            .iter()
            .filter(|request| matches!(request, StreamApiRequest::SendMessage { .. }))
            .count(),
        1,
        "notify=false should reuse the progress message for the final stream"
    );
    assert!(matches!(
        &requests[0],
        StreamApiRequest::SendMessage {
            text,
            disable_notification: true,
        } if !text.contains("Progress update") && text.contains("hel")
    ));
    assert!(
        !requests
            .iter()
            .any(|request| matches!(request, StreamApiRequest::DeleteMessage { .. }))
    );

    let final_edit = requests
        .iter()
        .rev()
        .find_map(|request| match request {
            StreamApiRequest::EditMessage { text } if text.contains("done") => Some(text),
            _ => None,
        })
        .expect("final edit message");
    assert!(final_edit.contains("<b>done</b>"));
    assert!(!final_edit.contains("**done**"));
    assert!(!final_edit.contains("hello"));
    assert!(!final_edit.contains("Progress update"));
}

#[tokio::test]
async fn send_stream_sends_remaining_final_chunks_on_done() {
    let long_final = format!("{}tail-marker", "word ".repeat(TELEGRAM_MAX_MESSAGE_LEN));
    let requests = run_stream_lifecycle(false, vec![
        (StreamEvent::Delta(long_final), Duration::from_millis(25)),
        (StreamEvent::Done, Duration::ZERO),
    ])
    .await;

    let sent_texts: Vec<&str> = requests
        .iter()
        .filter_map(|request| match request {
            StreamApiRequest::SendMessage { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        sent_texts.len() >= 2,
        "long streamed finals must continue after the edited first message"
    );
    assert!(sent_texts.iter().any(|text| text.contains("tail-marker")));
}

#[tokio::test]
async fn send_stream_sends_fallback_final_message_when_completion_edit_fails() {
    let final_text = "part one part two";
    let requests = run_stream_lifecycle_with_edit_failure(
        false,
        vec![
            (
                StreamEvent::Delta("part one".to_string()),
                Duration::from_millis(600),
            ),
            (
                StreamEvent::Delta(" part two".to_string()),
                Duration::from_millis(25),
            ),
            (StreamEvent::Done, Duration::ZERO),
        ],
        Some(final_text.to_string()),
    )
    .await;

    assert!(requests.iter().any(|request| matches!(
        request,
        StreamApiRequest::EditMessage { text } if text == final_text
    )));
    assert!(requests.iter().any(|request| matches!(
        request,
        StreamApiRequest::SendMessage { text, .. } if text == final_text
    )));
}

#[tokio::test]
async fn send_stream_reclassifies_live_draft_as_progress_then_reuses_for_final_when_notify_disabled()
 {
    let requests = run_stream_lifecycle(false, vec![
        (
            StreamEvent::Delta("**thinking**".to_string()),
            Duration::from_millis(600),
        ),
        (
            StreamEvent::ProgressDelta("**thinking**".to_string()),
            Duration::from_millis(600),
        ),
        (
            StreamEvent::Delta("**done**".to_string()),
            Duration::from_millis(600),
        ),
        (StreamEvent::Done, Duration::ZERO),
    ])
    .await;

    assert_eq!(
        requests
            .iter()
            .filter(|request| matches!(request, StreamApiRequest::SendMessage { .. }))
            .count(),
        1,
        "notify=false should keep draft, progress, and final in one message"
    );
    assert!(matches!(
        &requests[0],
        StreamApiRequest::SendMessage {
            text,
            disable_notification: true,
        } if text.contains("<b>thinking</b>")
    ));
    assert!(
        !requests
            .iter()
            .any(|request| matches!(request, StreamApiRequest::DeleteMessage { .. }))
    );

    let edit_texts: Vec<&str> = requests
        .iter()
        .filter_map(|request| match request {
            StreamApiRequest::EditMessage { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        edit_texts
            .iter()
            .any(|text| text.contains("<b>thinking</b>") && !text.contains("**thinking**"))
    );
    let final_edit = edit_texts
        .iter()
        .rev()
        .find(|text| text.contains("done"))
        .expect("final edit message");
    assert!(final_edit.contains("<b>done</b>"));
    assert!(!final_edit.contains("**done**"));
    assert!(!final_edit.contains("thinking"));
}

#[tokio::test]
async fn send_stream_keeps_code_fence_rendered_when_live_draft_becomes_progress() {
    let json = "JSON arguments:\n\n```json\n{\n  \"action\": \"add\"\n}\n```";
    let requests = run_stream_lifecycle(false, vec![
        (
            StreamEvent::Delta(json.to_string()),
            Duration::from_millis(600),
        ),
        (
            StreamEvent::ProgressDelta(json.to_string()),
            Duration::from_millis(600),
        ),
        (
            StreamEvent::Delta("done".to_string()),
            Duration::from_millis(600),
        ),
        (StreamEvent::Done, Duration::ZERO),
    ])
    .await;

    let progress_edit = requests
        .iter()
        .filter_map(|request| match request {
            StreamApiRequest::EditMessage { text } => Some(text.as_str()),
            _ => None,
        })
        .find(|text| text.contains("action"))
        .expect("progress edit containing JSON");

    assert!(progress_edit.contains("<pre"));
    assert!(progress_edit.contains("<code"));
    assert!(!progress_edit.contains("```json"));
}

#[tokio::test]
async fn send_stream_deletes_progress_and_sends_new_final_when_notify_enabled() {
    let requests = run_stream_lifecycle(true, vec![
        (
            StreamEvent::ProgressDelta("hel".to_string()),
            Duration::from_millis(25),
        ),
        (
            StreamEvent::Delta("**done**".to_string()),
            Duration::from_millis(25),
        ),
        (StreamEvent::Done, Duration::ZERO),
    ])
    .await;

    let sends: Vec<_> = requests
        .iter()
        .filter_map(|request| match request {
            StreamApiRequest::SendMessage {
                text,
                disable_notification,
            } => Some((text, *disable_notification)),
            _ => None,
        })
        .collect();
    assert_eq!(sends.len(), 2);
    assert!(sends[0].1, "progress message should be silent");
    assert!(!sends[0].0.contains("Progress update"));
    assert!(sends[0].0.contains("hel"));
    assert!(
        requests
            .iter()
            .any(|request| matches!(request, StreamApiRequest::DeleteMessage { message_id: 10 }))
    );
    assert!(!sends[1].1, "notify=true final message should notify");
    assert!(sends[1].0.contains("<b>done</b>"));
    assert!(!sends[1].0.contains("**done**"));
    assert!(!sends[1].0.contains("hel"));
}

#[tokio::test]
async fn send_html_fallback_sends_plain_text_without_raw_tags() {
    let recorded_requests = Arc::new(Mutex::new(Vec::<SendMessageRequest>::new()));
    let mock_api = MockTelegramApi {
        requests: Arc::clone(&recorded_requests),
    };
    let app = Router::new()
        .route("/{*path}", post(send_message_handler))
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

    let api_url = reqwest::Url::parse(&format!("http://{addr}/")).expect("parse api url");
    let bot = teloxide::Bot::new("test-token").set_api_url(api_url);

    let accounts: AccountStateMap = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let outbound = Arc::new(TelegramOutbound {
        accounts: Arc::clone(&accounts),
    });
    let account_id = "test-account";

    {
        let mut map = accounts.write().expect("accounts write lock");
        map.insert(account_id.to_string(), AccountState {
            bot: bot.clone(),
            bot_username: Some("test_bot".to_string()),
            account_id: account_id.to_string(),
            config: TelegramAccountConfig {
                token: Secret::new("test-token".to_string()),
                dm_policy: DmPolicy::Open,
                ..Default::default()
            },
            outbound: Arc::clone(&outbound),
            cancel: CancellationToken::new(),
            message_log: None,
            event_sink: None,
            otp: Mutex::new(OtpState::new(300)),
        });
    }

    outbound
        .send_html(
            account_id,
            "42",
            "<b>Hello</b> &amp; <i>world</i><br><code>&lt;ok&gt;</code>",
            None,
        )
        .await
        .expect("send html");

    {
        let requests = recorded_requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 2, "expected HTML send plus plain fallback");
        assert_eq!(requests[0].parse_mode.as_deref(), Some("HTML"));
        assert_eq!(
            requests[0].text,
            "<b>Hello</b> &amp; <i>world</i><br><code>&lt;ok&gt;</code>"
        );
        assert_eq!(requests[1].parse_mode, None);
        assert_eq!(requests[1].text, "Hello & world\n<ok>");
    }

    let _ = shutdown_tx.send(());
    server.await.expect("server join");
}

/// Regression test for <https://github.com/moltis-org/moltis/issues/947>.
///
/// teloxide-core 0.10.1 panicked in `PartSerializer::serialize_newtype_struct`
/// when serializing `ThreadId` (a newtype wrapping `MessageId`) in a multipart
/// request.  This happened whenever `send_document`, `send_voice`, or any media
/// method was called with `message_thread_id` set (i.e. forum/topic chats).
///
/// teloxide-core 0.13.0 (shipped with teloxide 0.17) fixes this by delegating
/// to `value.serialize(self)`.  This test verifies the fix by sending a
/// document to a topic chat target (`chat_id:thread_id` format), ensuring the
/// multipart request completes without panicking.
#[tokio::test]
async fn send_document_to_topic_chat_does_not_panic() {
    use {
        axum::{Router, body::Bytes, http::Uri, routing::post},
        moltis_channels::plugin::ChannelOutbound,
        moltis_common::types::{MediaAttachment, ReplyPayload},
    };

    // Mock Telegram API that returns a message result for every method.
    // This is sufficient for outbound media tests that only need a
    // non-error response.
    async fn api_handler(_uri: Uri, _body: Bytes) -> Json<serde_json::Value> {
        Json(serde_json::json!({
            "ok": true,
            "result": {
                "message_id": 1,
                "date": 0,
                "chat": { "id": -1001234, "type": "supergroup" },
                "text": ""
            }
        }))
    }

    let app = Router::new().route("/{*path}", post(api_handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("serve");
    });

    let api_url = reqwest::Url::parse(&format!("http://{addr}/")).expect("parse url");
    let bot = teloxide::Bot::new("test-token").set_api_url(api_url);

    let accounts: AccountStateMap = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let outbound = Arc::new(TelegramOutbound {
        accounts: Arc::clone(&accounts),
    });
    let account_id = "topic-test";

    {
        let mut map = accounts.write().expect("lock");
        map.insert(account_id.to_string(), AccountState {
            bot: bot.clone(),
            bot_username: Some("test_bot".into()),
            account_id: account_id.to_string(),
            config: TelegramAccountConfig {
                token: Secret::new("test-token".into()),
                ..Default::default()
            },
            outbound: Arc::clone(&outbound),
            cancel: CancellationToken::new(),
            message_log: None,
            event_sink: None,
            otp: Mutex::new(OtpState::new(300)),
        });
    }

    // Encode a small file as base64 data URI.
    let data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"OGG test data");
    let data_uri = format!("data:audio/ogg;base64,{data}");

    // Target is "chat_id:thread_id" — a forum topic chat.
    // With teloxide 0.13 (teloxide-core 0.10.1) this would panic in the
    // multipart serializer when ThreadId was serialized.
    let to = "-1001234:42";

    let payload = ReplyPayload {
        text: "voice test".into(),
        media: Some(MediaAttachment {
            url: data_uri,
            mime_type: "audio/ogg".into(),
            filename: Some("voice.ogg".into()),
        }),
        reply_to_id: None,
        silent: false,
    };

    // This must not panic.
    outbound
        .send_media(account_id, to, &payload, None)
        .await
        .expect("send_media to topic chat should succeed");

    let _ = shutdown_tx.send(());
    server.await.expect("server join");
}
