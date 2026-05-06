//! Log, session event, and network audit callbacks for the Swift FFI layer.

use std::{
    collections::HashMap,
    ffi::{CString, c_char},
    sync::OnceLock,
};

use {moltis_sessions::session_events::SessionEvent, serde::Serialize};

// ── Log callback for Swift ───────────────────────────────────────────────

/// Callback type for forwarding log events to Swift. Rust owns the
/// `log_json` pointer -- the callback must copy the data before returning.
#[allow(unsafe_code)]
pub(crate) type LogCallback = unsafe extern "C" fn(log_json: *const c_char);

pub(crate) static LOG_CALLBACK: OnceLock<LogCallback> = OnceLock::new();

/// JSON-serializable log event sent to Swift.
#[derive(Debug, Serialize)]
struct BridgeLogEvent<'a> {
    level: &'a str,
    target: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    fields: Option<&'a HashMap<&'a str, String>>,
}

pub(crate) fn emit_log(level: &str, target: &str, message: &str) {
    emit_log_with_fields(level, target, message, None);
}

#[allow(unsafe_code)]
pub(crate) fn emit_log_with_fields(
    level: &str,
    target: &str,
    message: &str,
    fields: Option<&HashMap<&str, String>>,
) {
    if let Some(callback) = LOG_CALLBACK.get() {
        let event = BridgeLogEvent {
            level,
            target,
            message,
            fields,
        };
        if let Ok(json) = serde_json::to_string(&event)
            && let Ok(c_str) = CString::new(json)
        {
            // SAFETY: c_str is valid NUL-terminated, callback copies
            // before returning, and we drop c_str afterwards.
            unsafe {
                callback(c_str.as_ptr());
            }
        }
    }
}

// ── Session event callback for Swift ─────────────────────────────────────

/// Callback type for forwarding session events to Swift.
/// Rust owns the `event_json` pointer -- the callback must copy the data
/// before returning.
#[allow(unsafe_code)]
pub(crate) type SessionEventCallback = unsafe extern "C" fn(event_json: *const c_char);

pub(crate) static SESSION_EVENT_CALLBACK: OnceLock<SessionEventCallback> = OnceLock::new();

/// JSON payload sent to Swift for each session event.
#[derive(Debug, Serialize)]
struct BridgeSessionEvent {
    kind: &'static str,
    #[serde(rename = "sessionKey")]
    session_key: String,
}

#[allow(unsafe_code)]
pub(crate) fn emit_session_event(event: &SessionEvent) {
    if let Some(callback) = SESSION_EVENT_CALLBACK.get() {
        let (kind, session_key) = match event {
            SessionEvent::Created { session_key } => ("created", session_key.clone()),
            SessionEvent::Deleted { session_key } => ("deleted", session_key.clone()),
            SessionEvent::Patched { session_key } => ("patched", session_key.clone()),
        };
        let payload = BridgeSessionEvent { kind, session_key };
        if let Ok(json) = serde_json::to_string(&payload)
            && let Ok(c_str) = CString::new(json)
        {
            // SAFETY: c_str is valid NUL-terminated, callback copies
            // before returning, and we drop c_str afterwards.
            unsafe {
                callback(c_str.as_ptr());
            }
        }
    }
}

// ── Network audit callback for Swift ─────────────────────────────────────

/// Callback type for forwarding network audit events to Swift.
/// Rust owns the `event_json` pointer -- the callback must copy the data
/// before returning.
#[allow(unsafe_code)]
pub(crate) type NetworkAuditCallback = unsafe extern "C" fn(event_json: *const c_char);

pub(crate) static NETWORK_AUDIT_CALLBACK: OnceLock<NetworkAuditCallback> = OnceLock::new();

/// JSON-serializable network audit event sent to Swift.
#[cfg(feature = "trusted-network")]
#[derive(Debug, Serialize)]
struct BridgeNetworkAuditEvent {
    domain: String,
    port: u16,
    protocol: String,
    action: String,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

#[allow(unsafe_code)]
#[cfg(feature = "trusted-network")]
pub(crate) fn emit_network_audit(entry: &moltis_network_filter::NetworkAuditEntry) {
    if let Some(callback) = NETWORK_AUDIT_CALLBACK.get() {
        let source = match &entry.approval_source {
            Some(moltis_network_filter::ApprovalSource::Config) => "config",
            Some(moltis_network_filter::ApprovalSource::Session) => "session",
            Some(moltis_network_filter::ApprovalSource::UserPrompt) => "user",
            None => "unknown",
        };
        let payload = BridgeNetworkAuditEvent {
            domain: entry.domain.clone(),
            port: entry.port,
            protocol: entry.protocol.to_string(),
            action: entry.action.to_string(),
            source: source.to_owned(),
            method: entry.method.clone(),
            url: entry.url.clone(),
        };
        if let Ok(json) = serde_json::to_string(&payload)
            && let Ok(c_str) = CString::new(json)
        {
            // SAFETY: c_str is valid NUL-terminated, callback copies
            // before returning, and we drop c_str afterwards.
            unsafe {
                callback(c_str.as_ptr());
            }
        }
    }
}
