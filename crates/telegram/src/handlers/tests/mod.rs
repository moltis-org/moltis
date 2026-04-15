#![allow(clippy::unwrap_used, clippy::expect_used)]

use {
    super::{otp::OTP_CHALLENGE_MSG, *},
    std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    },
};

use {
    async_trait::async_trait,
    axum::{Json, Router, body::Bytes, extract::State, http::Uri, routing::post},
    moltis_channels::{
        ChannelAttachment, ChannelDocumentFile, ChannelEvent, ChannelEventSink, ChannelMessageKind,
        ChannelMessageMeta, ChannelReplyTarget, Error as ChannelError, Result, SavedChannelFile,
        gating::DmPolicy,
    },
    secrecy::Secret,
    serde::{Deserialize, Serialize},
    serde_json::json,
    tokio::sync::oneshot,
    tokio_util::sync::CancellationToken,
};

use crate::{
    config::TelegramAccountConfig,
    otp::OtpState,
    outbound::TelegramOutbound,
    state::{AccountState, AccountStateMap},
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum TelegramApiMethod {
    SendMessage,
    SendChatAction,
    GetFile,
    Other(String),
}

impl TelegramApiMethod {
    fn from_path(path: &str) -> Self {
        let method = path.rsplit('/').next().unwrap_or_default();
        match method {
            "SendMessage" | "sendMessage" => Self::SendMessage,
            "SendChatAction" | "sendChatAction" => Self::SendChatAction,
            "GetFile" | "getFile" => Self::GetFile,
            _ => Self::Other(method.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
enum CapturedTelegramRequest {
    SendMessage(SendMessageRequest),
    SendChatAction(SendChatActionRequest),
    Other {
        method: TelegramApiMethod,
        raw_body: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct SendMessageRequest {
    chat_id: i64,
    text: String,
    #[serde(default)]
    parse_mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SendChatActionRequest {
    chat_id: i64,
    action: String,
}

#[derive(Debug, Serialize)]
struct TelegramApiResponse {
    ok: bool,
    result: TelegramApiResult,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum TelegramApiResult {
    Message(TelegramMessageResult),
    File(TelegramFileResult),
    Bool(bool),
}

#[derive(Debug, Serialize)]
struct TelegramFileResult {
    file_id: String,
    file_unique_id: String,
    file_path: String,
}

#[derive(Debug, Serialize)]
struct TelegramChat {
    id: i64,
    #[serde(rename = "type")]
    chat_type: String,
}

#[derive(Debug, Serialize)]
struct TelegramMessageResult {
    message_id: i64,
    date: i64,
    chat: TelegramChat,
    text: String,
}

#[derive(Clone)]
struct MockTelegramApi {
    requests: Arc<Mutex<Vec<CapturedTelegramRequest>>>,
}

async fn telegram_api_handler(
    State(state): State<MockTelegramApi>,
    uri: Uri,
    body: Bytes,
) -> Json<TelegramApiResponse> {
    let method = TelegramApiMethod::from_path(uri.path());
    let raw_body = String::from_utf8_lossy(&body).to_string();

    let captured = match method.clone() {
        TelegramApiMethod::SendMessage => {
            match serde_json::from_slice::<SendMessageRequest>(&body) {
                Ok(req) => CapturedTelegramRequest::SendMessage(req),
                Err(_) => CapturedTelegramRequest::Other { method, raw_body },
            }
        },
        TelegramApiMethod::SendChatAction => {
            match serde_json::from_slice::<SendChatActionRequest>(&body) {
                Ok(req) => CapturedTelegramRequest::SendChatAction(req),
                Err(_) => CapturedTelegramRequest::Other { method, raw_body },
            }
        },
        TelegramApiMethod::GetFile | TelegramApiMethod::Other(_) => {
            CapturedTelegramRequest::Other { method, raw_body }
        },
    };

    state.requests.lock().expect("lock requests").push(captured);

    match TelegramApiMethod::from_path(uri.path()) {
        TelegramApiMethod::SendMessage => Json(TelegramApiResponse {
            ok: true,
            result: TelegramApiResult::Message(TelegramMessageResult {
                message_id: 1,
                date: 0,
                chat: TelegramChat {
                    id: 42,
                    chat_type: "private".to_string(),
                },
                text: "ok".to_string(),
            }),
        }),
        TelegramApiMethod::GetFile => Json(TelegramApiResponse {
            ok: true,
            result: TelegramApiResult::File(TelegramFileResult {
                file_id: "test-file-id".to_string(),
                file_unique_id: "test-unique-id".to_string(),
                file_path: "voice/test-voice.ogg".to_string(),
            }),
        }),
        TelegramApiMethod::SendChatAction | TelegramApiMethod::Other(_) => {
            Json(TelegramApiResponse {
                ok: true,
                result: TelegramApiResult::Bool(true),
            })
        },
    }
}

#[derive(Default)]
struct MockSink {
    dispatch_calls: std::sync::atomic::AtomicUsize,
    dispatched_texts: Mutex<Vec<String>>,
    dispatched_with_attachments: Mutex<Vec<DispatchedAttachment>>,
    dispatched_documents: Mutex<Vec<Option<Vec<ChannelDocumentFile>>>>,
    stt_available: bool,
    transcription_result: Mutex<Option<Result<String>>>,
}

#[derive(Debug, Clone)]
struct DispatchedAttachment {
    text: String,
    media_types: Vec<String>,
    sizes: Vec<usize>,
}

fn escaped_telegram_reply_text(text: &str) -> String {
    text.replace('>', "&gt;")
}

fn is_escaped_reply_to_chat(message: &SendMessageRequest, chat_id: i64, text: &str) -> bool {
    message.chat_id == chat_id && message.text == escaped_telegram_reply_text(text)
}

impl MockSink {
    fn with_stt(transcription: Result<String>) -> Self {
        Self::with_voice_stt(true, Some(transcription))
    }

    fn with_voice_stt(stt_available: bool, transcription: Option<Result<String>>) -> Self {
        Self {
            stt_available,
            transcription_result: Mutex::new(transcription),
            ..Default::default()
        }
    }
}

#[async_trait]
impl ChannelEventSink for MockSink {
    async fn emit(&self, _event: ChannelEvent) {}

    async fn dispatch_to_chat(
        &self,
        text: &str,
        _reply_to: ChannelReplyTarget,
        meta: ChannelMessageMeta,
    ) {
        self.dispatch_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.dispatched_texts
            .lock()
            .expect("lock")
            .push(text.to_string());
        self.dispatched_documents
            .lock()
            .expect("lock")
            .push(meta.documents);
    }

    async fn dispatch_to_chat_with_attachments(
        &self,
        text: &str,
        attachments: Vec<ChannelAttachment>,
        _reply_to: ChannelReplyTarget,
        meta: ChannelMessageMeta,
    ) {
        let media_types = attachments
            .iter()
            .map(|attachment| attachment.media_type.clone())
            .collect();
        let sizes = attachments
            .iter()
            .map(|attachment| attachment.data.len())
            .collect();
        self.dispatched_with_attachments
            .lock()
            .expect("lock")
            .push(DispatchedAttachment {
                text: text.to_string(),
                media_types,
                sizes,
            });
        self.dispatched_documents
            .lock()
            .expect("lock")
            .push(meta.documents);
    }

    async fn dispatch_command(
        &self,
        _command: &str,
        _reply_to: ChannelReplyTarget,
        _sender_id: Option<&str>,
    ) -> Result<String> {
        Ok(String::new())
    }

    async fn request_disable_account(&self, _channel_type: &str, _account_id: &str, _reason: &str) {
    }

    async fn save_channel_attachment(
        &self,
        _file_data: &[u8],
        filename: &str,
        _reply_to: &ChannelReplyTarget,
    ) -> Option<SavedChannelFile> {
        Some(SavedChannelFile {
            filename: filename.to_string(),
            media_ref: format!("media/mock/{filename}"),
            absolute_path: format!("/tmp/mock-saved/{filename}"),
        })
    }

    async fn transcribe_voice(&self, _audio_data: &[u8], _format: &str) -> Result<String> {
        self.transcription_result
            .lock()
            .expect("lock")
            .take()
            .unwrap_or_else(|| {
                Err(ChannelError::unavailable(
                    "transcribe should not be called when STT unavailable",
                ))
            })
    }

    async fn voice_stt_available(&self) -> bool {
        self.stt_available
    }
}

mod location;
mod media;
mod session;
mod voice;
