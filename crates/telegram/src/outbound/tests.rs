#![allow(clippy::unwrap_used, clippy::expect_used)]
use {
    axum::{Json, Router, extract::State, http::StatusCode, routing::post},
    moltis_channels::{gating::DmPolicy, plugin::ChannelOutbound},
    secrecy::Secret,
    serde::{Deserialize, Serialize},
    std::{
        collections::HashMap,
        sync::{Arc, Mutex},
        time::Duration,
    },
    teloxide::{ApiError, RequestError},
    tokio::sync::oneshot,
    tokio_util::sync::CancellationToken,
};

use crate::{
    config::TelegramAccountConfig,
    otp::OtpState,
    state::{AccountState, AccountStateMap},
};

use super::{
    TelegramOutbound,
    formatting::telegram_html_to_plain_text,
    retry::{is_message_not_modified_error, retry_after_duration},
    stream::{has_reached_stream_min_initial_chars, should_send_stream_completion_notification},
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
    let err = RequestError::Io(std::io::Error::other("boom"));
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
    let err = RequestError::Io(std::io::Error::other("boom"));
    assert!(!is_message_not_modified_error(&err));
}

#[test]
fn stream_min_initial_chars_uses_character_count() {
    assert!(has_reached_stream_min_initial_chars("hello", 5));
    assert!(has_reached_stream_min_initial_chars(
        "\u{1F642}\u{1F642}\u{1F642}",
        3
    ));
    assert!(!has_reached_stream_min_initial_chars(
        "\u{1F642}\u{1F642}\u{1F642}",
        4
    ));
}

#[test]
fn stream_completion_notification_requires_opt_in() {
    assert!(!should_send_stream_completion_notification(
        false, true, false
    ));
}

#[test]
fn stream_completion_notification_skips_when_no_text() {
    assert!(!should_send_stream_completion_notification(
        true, false, false
    ));
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

#[test]
fn stream_completion_notification_skips_when_already_notified_by_chunks() {
    assert!(!should_send_stream_completion_notification(
        true, true, true
    ));
}

#[test]
fn stream_completion_notification_enabled_when_needed() {
    assert!(should_send_stream_completion_notification(
        true, true, false
    ));
}
