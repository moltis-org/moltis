//! Session FFI exports: list, create, switch, and session-scoped streaming chat.

use std::{
    ffi::{CString, c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
};

use {
    moltis_agents::model::{ChatMessage as AgentChatMessage, StreamEvent, Usage, UserContent},
    moltis_sessions::message::PersistedMessage,
    tokio_stream::StreamExt,
};

use crate::{
    callbacks::emit_log,
    chat::{BridgeStreamEvent, StreamCallback, StreamCallbackCtx, resolve_provider_for_model},
    helpers::{
        encode_error, encode_json, parse_ffi_request, read_c_string, record_call, trace_call,
        with_ffi_boundary,
    },
    state::BRIDGE,
    types::{
        BridgeSessionEntry, BridgeSessionHistory, CreateSessionRequest, SessionChatRequest,
        SwitchSessionRequest,
    },
};

// ── Session FFI exports ──────────────────────────────────────────────────

/// Returns JSON array of all session entries (sorted by created_at ASC, matching web UI).
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_list_sessions() -> *mut c_char {
    record_call("moltis_list_sessions");
    trace_call("moltis_list_sessions");

    with_ffi_boundary(|| {
        let all = BRIDGE.runtime.block_on(BRIDGE.session_metadata.list());
        let entries: Vec<BridgeSessionEntry> = all.iter().map(BridgeSessionEntry::from).collect();
        emit_log(
            "DEBUG",
            "bridge.sessions",
            &format!("Listed {} sessions", entries.len()),
        );
        encode_json(&entries)
    })
}

/// Switches to a session by key. Returns entry + message history.
/// If the session doesn't exist yet, it will be created.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_switch_session(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_switch_session");
    trace_call("moltis_switch_session");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SwitchSessionRequest>(
            "moltis_switch_session",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        // Ensure metadata entry exists.
        if let Err(e) = BRIDGE
            .runtime
            .block_on(BRIDGE.session_metadata.upsert(&request.key, None))
        {
            emit_log(
                "WARN",
                "bridge.sessions",
                &format!("Failed to upsert metadata: {e}"),
            );
        }

        // Read message history from JSONL.
        let messages = match BRIDGE
            .runtime
            .block_on(BRIDGE.session_store.read(&request.key))
        {
            Ok(msgs) => msgs,
            Err(e) => {
                emit_log(
                    "WARN",
                    "bridge.sessions",
                    &format!("Failed to read session: {e}"),
                );
                vec![]
            },
        };

        let entry = BRIDGE
            .runtime
            .block_on(BRIDGE.session_metadata.get(&request.key))
            .map(|e| BridgeSessionEntry::from(&e));

        match entry {
            Some(entry) => {
                emit_log(
                    "INFO",
                    "bridge.sessions",
                    &format!(
                        "Switched to session '{}' ({} messages)",
                        request.key,
                        messages.len()
                    ),
                );
                encode_json(&BridgeSessionHistory { entry, messages })
            },
            None => encode_error(
                "session_not_found",
                &format!("Session '{}' not found", request.key),
            ),
        }
    })
}

/// Creates a new session with an optional label. Returns the entry.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_create_session(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_create_session");
    trace_call("moltis_create_session");

    with_ffi_boundary(|| {
        let request: CreateSessionRequest = if request_json.is_null() {
            CreateSessionRequest { label: None }
        } else {
            match read_c_string(request_json) {
                Ok(raw) => match serde_json::from_str(&raw) {
                    Ok(r) => r,
                    Err(e) => return encode_error("invalid_json", &e.to_string()),
                },
                Err(msg) => return encode_error("null_pointer_or_invalid_utf8", &msg),
            }
        };

        let key = format!("session:{}", uuid::Uuid::new_v4());
        let label = request.label.unwrap_or_else(|| "New Session".to_owned());

        match BRIDGE
            .runtime
            .block_on(BRIDGE.session_metadata.upsert(&key, Some(label)))
        {
            Ok(entry) => {
                emit_log(
                    "INFO",
                    "bridge.sessions",
                    &format!("Created session '{}'", key),
                );
                encode_json(&BridgeSessionEntry::from(&entry))
            },
            Err(e) => encode_error("create_failed", &format!("Failed to create session: {e}")),
        }
    })
}

/// Streaming chat within a session. Persists user message before streaming,
/// persists assistant message when done. Events delivered via callback.
///
/// # Safety
///
/// Same requirements as `moltis_chat_stream`.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moltis_session_chat_stream(
    request_json: *const c_char,
    callback: StreamCallback,
    user_data: *mut c_void,
) {
    record_call("moltis_session_chat_stream");
    trace_call("moltis_session_chat_stream");

    let send_error = |msg: String| {
        let event = BridgeStreamEvent::Error { message: msg };
        let json = encode_json(&event);
        if let Ok(c_str) = CString::new(json) {
            unsafe {
                callback(c_str.as_ptr(), user_data);
            }
        }
    };

    let raw = match read_c_string(request_json) {
        Ok(value) => value,
        Err(message) => {
            send_error(message);
            return;
        },
    };

    let request = match serde_json::from_str::<SessionChatRequest>(&raw) {
        Ok(r) => r,
        Err(e) => {
            send_error(e.to_string());
            return;
        },
    };

    let provider = match resolve_provider_for_model(request.model.as_deref()) {
        Some(p) => p,
        None => {
            send_error("No LLM provider configured".to_owned());
            return;
        },
    };

    let session_key = request.session_key.clone();

    // Persist user message.
    let user_msg = PersistedMessage::user(&request.message);
    let user_value = user_msg.to_value();
    if let Err(e) = BRIDGE
        .runtime
        .block_on(BRIDGE.session_store.append(&session_key, &user_value))
    {
        emit_log(
            "WARN",
            "bridge.session_chat",
            &format!("Failed to persist user message: {e}"),
        );
    }

    // Update metadata.
    BRIDGE.runtime.block_on(async {
        let _ = BRIDGE.session_metadata.upsert(&session_key, None).await;
        let msg_count = BRIDGE
            .session_store
            .read(&session_key)
            .await
            .map(|m| m.len() as u32)
            .unwrap_or(0);
        BRIDGE.session_metadata.touch(&session_key, msg_count).await;
    });

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
        "bridge.session_chat",
        &format!(
            "Starting session stream: session={} provider={}/{}",
            session_key, provider_name, model_id
        ),
    );

    BRIDGE.runtime.spawn(async move {
        let start = std::time::Instant::now();
        let result = catch_unwind(AssertUnwindSafe(|| provider.stream(messages)));

        let mut stream = match result {
            Ok(s) => s,
            Err(_) => {
                ctx.send(&BridgeStreamEvent::Error {
                    message: "panic during stream creation".to_owned(),
                });
                return;
            },
        };

        let mut usage = Usage::default();
        let mut full_text = String::new();

        while let Some(event) = stream.next().await {
            match event {
                StreamEvent::Delta(text) => {
                    full_text.push_str(&text);
                    ctx.send(&BridgeStreamEvent::Delta { text });
                },
                StreamEvent::Done(u) => {
                    usage = u;
                    break;
                },
                StreamEvent::Error(message) => {
                    ctx.send(&BridgeStreamEvent::Error { message });
                    return;
                },
                _ => {},
            }
        }

        let elapsed = start.elapsed().as_millis() as u64;

        // Persist assistant message.
        let assistant_msg = PersistedMessage::assistant(
            &full_text,
            &model_id,
            &provider_name,
            usage.input_tokens,
            usage.output_tokens,
            None, // audio
        );
        let assistant_value = assistant_msg.to_value();
        if let Err(e) = BRIDGE
            .session_store
            .append(&session_key, &assistant_value)
            .await
        {
            emit_log(
                "WARN",
                "bridge.session_chat",
                &format!("Failed to persist assistant message: {e}"),
            );
        }

        // Update metadata in SQLite.
        let msg_count = BRIDGE
            .session_store
            .read(&session_key)
            .await
            .map(|m| m.len() as u32)
            .unwrap_or(0);
        BRIDGE.session_metadata.touch(&session_key, msg_count).await;
        BRIDGE
            .session_metadata
            .set_model(&session_key, Some(model_id.clone()))
            .await;

        ctx.send(&BridgeStreamEvent::Done {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            duration_ms: elapsed,
            model: Some(model_id),
            provider: Some(provider_name),
        });
    });
}

// ── Tests ────────────────────────────────────────────────────────────────

#[allow(unsafe_code)]
#[cfg(test)]
mod tests {
    use std::ffi::{CString, c_char};

    use serde_json::Value;

    use super::*;

    fn text_from_ptr(ptr: *mut c_char) -> String {
        assert!(!ptr.is_null(), "ffi returned null pointer");
        let owned = unsafe { CString::from_raw(ptr) };
        match owned.into_string() {
            Ok(text) => text,
            Err(error) => panic!("failed to decode UTF-8 from ffi pointer: {error}"),
        }
    }

    fn json_from_ptr(ptr: *mut c_char) -> Value {
        let text = text_from_ptr(ptr);
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => value,
            Err(error) => panic!("failed to parse ffi json payload: {error}; payload={text}"),
        }
    }

    #[test]
    fn list_sessions_returns_array() {
        let payload = json_from_ptr(moltis_list_sessions());
        assert!(
            payload.as_array().is_some(),
            "list_sessions should return a JSON array"
        );
    }

    #[test]
    fn create_and_switch_session() {
        // Create a session with a label.
        let request = r#"{"label":"Test Session"}"#;
        let c_request = CString::new(request).unwrap_or_else(|e| panic!("{e}"));
        let payload = json_from_ptr(moltis_create_session(c_request.as_ptr()));

        let key = payload
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(
            key.starts_with("session:"),
            "created session key should start with 'session:'"
        );
        assert_eq!(
            payload.get("label").and_then(Value::as_str),
            Some("Test Session"),
        );

        // Switch to the created session.
        let switch_request = serde_json::json!({"key": key}).to_string();
        let c_switch = CString::new(switch_request).unwrap_or_else(|e| panic!("{e}"));
        let history = json_from_ptr(moltis_switch_session(c_switch.as_ptr()));

        assert!(history.get("entry").is_some(), "switch should return entry");
        assert!(
            history.get("messages").and_then(Value::as_array).is_some(),
            "switch should return messages array"
        );
    }

    #[test]
    fn create_session_with_null_uses_defaults() {
        let payload = json_from_ptr(moltis_create_session(std::ptr::null()));

        let key = payload
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(
            key.starts_with("session:"),
            "session key should start with 'session:'"
        );
        assert_eq!(
            payload.get("label").and_then(Value::as_str),
            Some("New Session"),
        );
    }
}
