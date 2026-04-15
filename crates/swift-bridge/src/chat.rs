//! Chat logic: provider resolution, non-streaming chat, and streaming support.

use std::{
    ffi::{CString, c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
};

use {
    moltis_agents::model::{
        ChatMessage as AgentChatMessage, LlmProvider, StreamEvent, Usage, UserContent,
    },
    serde::Serialize,
    tokio_stream::StreamExt,
};

use crate::{
    callbacks::emit_log,
    helpers::{
        build_validation_summary, config_dir_string, encode_json, record_call, record_error,
        trace_call,
    },
    state::BRIDGE,
    types::{ChatRequest, ChatResponse},
};

// ── Provider resolution ──────────────────────────────────────────────────

pub(crate) fn resolve_provider(request: &ChatRequest) -> Option<std::sync::Arc<dyn LlmProvider>> {
    resolve_provider_for_model(request.model.as_deref())
}

pub(crate) fn resolve_provider_for_model(
    model: Option<&str>,
) -> Option<std::sync::Arc<dyn LlmProvider>> {
    let registry = BRIDGE.registry.read().unwrap_or_else(|e| e.into_inner());

    // Try explicit model first
    if let Some(model_id) = model
        && let Some(provider) = registry.get(model_id)
    {
        emit_log(
            "DEBUG",
            "bridge",
            &format!(
                "Resolved provider for model={}: {}",
                model_id,
                provider.name()
            ),
        );
        return Some(provider);
    }

    // Fall back to first available provider
    let result = registry.first();
    if let Some(ref p) = result {
        emit_log(
            "DEBUG",
            "bridge",
            &format!("Using first available provider: {} ({})", p.name(), p.id()),
        );
    } else {
        emit_log("WARN", "bridge", "No provider available in registry");
    }
    result
}

pub(crate) fn build_chat_response(request: ChatRequest) -> String {
    emit_log(
        "INFO",
        "bridge.chat",
        &format!(
            "Chat request: model={:?} msg_len={}",
            request.model,
            request.message.len()
        ),
    );
    let validation = build_validation_summary(request.config_toml.as_deref());

    let (reply, model, provider_name, input_tokens, output_tokens, duration_ms) =
        match resolve_provider(&request) {
            Some(provider) => {
                let model_id = provider.id().to_string();
                let provider_name = provider.name().to_string();
                let messages = vec![AgentChatMessage::User {
                    content: UserContent::text(&request.message),
                    name: None,
                }];

                emit_log(
                    "DEBUG",
                    "bridge.chat",
                    &format!("Calling {}/{}", provider_name, model_id),
                );
                let start = std::time::Instant::now();
                match BRIDGE.runtime.block_on(provider.complete(&messages, &[])) {
                    Ok(response) => {
                        let elapsed = start.elapsed().as_millis() as u64;
                        let text = response
                            .text
                            .unwrap_or_else(|| "(empty response)".to_owned());
                        let in_tok = response.usage.input_tokens;
                        let out_tok = response.usage.output_tokens;
                        emit_log(
                            "INFO",
                            "bridge.chat",
                            &format!(
                                "Response: {}ms in={} out={} provider={}",
                                elapsed, in_tok, out_tok, provider_name
                            ),
                        );
                        (
                            text,
                            Some(model_id),
                            Some(provider_name),
                            Some(in_tok),
                            Some(out_tok),
                            Some(elapsed),
                        )
                    },
                    Err(error) => {
                        let msg = format!("LLM error: {error}");
                        emit_log("ERROR", "bridge.chat", &msg);
                        (msg, Some(model_id), Some(provider_name), None, None, None)
                    },
                }
            },
            None => {
                let msg = "No LLM provider configured".to_owned();
                emit_log("WARN", "bridge.chat", &msg);
                (
                    format!("{msg}. Rust bridge received: {}", request.message),
                    None,
                    None,
                    None,
                    None,
                    None,
                )
            },
        };

    let response = ChatResponse {
        reply,
        model,
        provider: provider_name,
        config_dir: config_dir_string(),
        default_soul: moltis_config::DEFAULT_SOUL.to_owned(),
        validation,
        input_tokens,
        output_tokens,
        duration_ms,
    };
    encode_json(&response)
}

// ── Streaming support ────────────────────────────────────────────────────

/// Callback type for streaming events. Rust owns the `event_json` pointer --
/// the callback must copy the data before returning; Rust drops it afterwards.
#[allow(unsafe_code)]
pub(crate) type StreamCallback =
    unsafe extern "C" fn(event_json: *const c_char, user_data: *mut c_void);

/// JSON-serializable event sent to Swift via the callback.
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub(crate) enum BridgeStreamEvent {
    #[serde(rename = "delta")]
    Delta { text: String },
    #[serde(rename = "done")]
    Done {
        input_tokens: u32,
        output_tokens: u32,
        duration_ms: u64,
        model: Option<String>,
        provider: Option<String>,
    },
    #[serde(rename = "error")]
    Error { message: String },
}

/// Bundle of callback + user_data that can cross the `tokio::spawn` boundary.
///
/// # Safety
///
/// The Swift side guarantees that `user_data` remains valid until a terminal
/// event (done/error) is received, and the callback function pointer is
/// stable for the lifetime of the stream. The callback dispatches to the
/// main thread so there is no concurrent access.
pub(crate) struct StreamCallbackCtx {
    pub callback: StreamCallback,
    pub user_data: *mut c_void,
}

// SAFETY: See struct doc -- Swift retains `StreamContext` via
// `Unmanaged.passRetained` and the callback itself is a plain function pointer.
#[allow(unsafe_code)]
unsafe impl Send for StreamCallbackCtx {}

#[allow(unsafe_code)]
impl StreamCallbackCtx {
    pub fn send(&self, event: &BridgeStreamEvent) {
        let json = encode_json(event);
        if let Ok(c_str) = CString::new(json) {
            // SAFETY: `c_str` is a valid NUL-terminated C string, `user_data`
            // is retained by the Swift caller, and the callback copies the
            // string contents before returning. We drop `c_str` afterwards.
            unsafe {
                (self.callback)(c_str.as_ptr(), self.user_data);
            }
        }
    }
}

/// Start a streaming LLM chat. Events are delivered via `callback`. The
/// function returns immediately; the stream runs on the bridge's tokio
/// runtime. The caller must keep `user_data` alive until a terminal event
/// (done or error) is delivered.
///
/// # Safety
///
/// * `request_json` must be a valid NUL-terminated C string.
/// * `callback` must be a valid function pointer that remains valid for the
///   lifetime of the stream.
/// * `user_data` must remain valid until the callback receives a terminal
///   event (done or error).
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moltis_chat_stream(
    request_json: *const c_char,
    callback: StreamCallback,
    user_data: *mut c_void,
) {
    record_call("moltis_chat_stream");
    trace_call("moltis_chat_stream");

    // Helper to send an error event before `ctx` is constructed.
    let send_error = |msg: String| {
        let event = BridgeStreamEvent::Error { message: msg };
        let json = encode_json(&event);
        if let Ok(c_str) = CString::new(json) {
            // SAFETY: caller guarantees valid callback + user_data.
            unsafe {
                callback(c_str.as_ptr(), user_data);
            }
        }
    };

    // Parse request synchronously on the calling thread so errors are
    // reported immediately via callback (no need to spawn).
    let raw = match crate::helpers::read_c_string(request_json) {
        Ok(value) => value,
        Err(message) => {
            record_error("moltis_chat_stream", "null_pointer_or_invalid_utf8");
            send_error(message);
            return;
        },
    };

    let request = match serde_json::from_str::<ChatRequest>(&raw) {
        Ok(request) => request,
        Err(error) => {
            record_error("moltis_chat_stream", "invalid_json");
            send_error(error.to_string());
            return;
        },
    };

    let provider = match resolve_provider(&request) {
        Some(p) => p,
        None => {
            send_error("No LLM provider configured".to_owned());
            return;
        },
    };

    let model_id = provider.id().to_string();
    let provider_name = provider.name().to_string();
    let messages = vec![AgentChatMessage::User {
        content: UserContent::text(&request.message),
        name: None,
    }];

    let ctx = StreamCallbackCtx {
        callback,
        user_data,
    };

    emit_log(
        "INFO",
        "bridge.stream",
        &format!("Starting stream: {}/{}", provider_name, model_id),
    );

    BRIDGE.runtime.spawn(async move {
        let start = std::time::Instant::now();

        let result = catch_unwind(AssertUnwindSafe(|| provider.stream(messages)));

        let mut stream = match result {
            Ok(s) => s,
            Err(_) => {
                emit_log("ERROR", "bridge.stream", "Panic during stream creation");
                ctx.send(&BridgeStreamEvent::Error {
                    message: "panic during stream creation".to_owned(),
                });
                return;
            },
        };

        let mut usage = Usage::default();
        let mut delta_count: u32 = 0;

        while let Some(event) = stream.next().await {
            match event {
                StreamEvent::Delta(text) => {
                    delta_count += 1;
                    ctx.send(&BridgeStreamEvent::Delta { text });
                },
                StreamEvent::Done(u) => {
                    usage = u;
                    break;
                },
                StreamEvent::Error(message) => {
                    emit_log(
                        "ERROR",
                        "bridge.stream",
                        &format!("Stream error: {message}"),
                    );
                    ctx.send(&BridgeStreamEvent::Error { message });
                    return;
                },
                // Ignore tool-call and reasoning events for chat UI.
                _ => {},
            }
        }

        let elapsed = start.elapsed().as_millis() as u64;
        emit_log(
            "INFO",
            "bridge.stream",
            &format!(
                "Stream done: {}ms deltas={} in={} out={} provider={}",
                elapsed, delta_count, usage.input_tokens, usage.output_tokens, provider_name
            ),
        );
        ctx.send(&BridgeStreamEvent::Done {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            duration_ms: elapsed,
            model: Some(model_id),
            provider: Some(provider_name),
        });
    });
}
