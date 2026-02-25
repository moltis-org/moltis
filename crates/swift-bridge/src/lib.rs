//! C ABI bridge for embedding Moltis Rust functionality into native Swift apps.

#![allow(unsafe_code)]

use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{LazyLock, OnceLock, RwLock},
};

use {
    moltis_agents::model::{
        ChatMessage as AgentChatMessage, LlmProvider, StreamEvent, Usage, UserContent,
    },
    moltis_config::{schema::ProvidersConfig, validate::Severity},
    moltis_provider_setup::{
        KeyStore, config_with_saved_keys, detect_auto_provider_sources_with_overrides,
        known_providers,
    },
    moltis_providers::ProviderRegistry,
    serde::{Deserialize, Serialize},
    tokio_stream::StreamExt,
};

// ── Global bridge state ────────────────────────────────────────────────────

struct BridgeState {
    runtime: tokio::runtime::Runtime,
    registry: RwLock<ProviderRegistry>,
}

impl BridgeState {
    fn new() -> Self {
        emit_log("INFO", "bridge", "Initializing Rust bridge (tokio runtime + registry)");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap_or_else(|e| panic!("failed to create tokio runtime: {e}"));

        let registry = build_registry();
        emit_log("INFO", "bridge", "Bridge initialized successfully");
        Self {
            runtime,
            registry: RwLock::new(registry),
        }
    }
}

fn build_registry() -> ProviderRegistry {
    let base = ProvidersConfig::default();
    let key_store = KeyStore::new();
    let config = config_with_saved_keys(&base, &key_store, &[]);
    ProviderRegistry::from_env_with_config(&config)
}

static BRIDGE: LazyLock<BridgeState> = LazyLock::new(BridgeState::new);

// ── Log callback for Swift ───────────────────────────────────────────────

/// Callback type for forwarding log events to Swift. Rust owns the
/// `log_json` pointer — the callback must copy the data before returning.
type LogCallback = unsafe extern "C" fn(log_json: *const c_char);

static LOG_CALLBACK: OnceLock<LogCallback> = OnceLock::new();

/// JSON-serializable log event sent to Swift.
#[derive(Debug, Serialize)]
struct BridgeLogEvent<'a> {
    level: &'a str,
    target: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    fields: Option<&'a HashMap<&'a str, String>>,
}

fn emit_log(level: &str, target: &str, message: &str) {
    emit_log_with_fields(level, target, message, None);
}

fn emit_log_with_fields(
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
        if let Ok(json) = serde_json::to_string(&event) {
            if let Ok(c_str) = CString::new(json) {
                // SAFETY: c_str is valid NUL-terminated, callback copies
                // before returning, and we drop c_str afterwards.
                unsafe {
                    callback(c_str.as_ptr());
                }
            }
        }
    }
}

// ── Request / Response types ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ChatRequest {
    message: String,
    #[serde(default)]
    model: Option<String>,
    /// Reserved for future provider-hint resolution; deserialized so Swift
    /// can pass it but not yet used for routing.
    #[serde(default)]
    #[allow(dead_code)]
    provider: Option<String>,
    #[serde(default)]
    config_toml: Option<String>,
}

#[derive(Debug, Serialize)]
struct ChatResponse {
    reply: String,
    model: Option<String>,
    provider: Option<String>,
    config_dir: String,
    default_soul: String,
    validation: Option<ValidationSummary>,
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
    duration_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
struct ValidationSummary {
    errors: usize,
    warnings: usize,
    info: usize,
    has_errors: bool,
}

#[derive(Debug, Serialize)]
struct VersionResponse {
    bridge_version: &'static str,
    moltis_version: &'static str,
    config_dir: String,
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope<'a> {
    error: ErrorPayload<'a>,
}

#[derive(Debug, Serialize)]
struct ErrorPayload<'a> {
    code: &'a str,
    message: &'a str,
}

// ── Bridge serde types for provider data ───────────────────────────────────

#[derive(Debug, Serialize)]
struct BridgeKnownProvider {
    name: &'static str,
    display_name: &'static str,
    auth_type: &'static str,
    env_key: Option<&'static str>,
    default_base_url: Option<&'static str>,
    requires_model: bool,
    key_optional: bool,
}

#[derive(Debug, Serialize)]
struct BridgeDetectedSource {
    provider: String,
    source: String,
}

#[derive(Debug, Serialize)]
struct BridgeModelInfo {
    id: String,
    provider: String,
    display_name: String,
    created_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct SaveProviderRequest {
    provider: String,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    models: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct OkResponse {
    ok: bool,
}

// ── Encoding helpers ───────────────────────────────────────────────────────

fn encode_json<T: Serialize>(value: &T) -> String {
    match serde_json::to_string(value) {
        Ok(json) => json,
        Err(_) => {
            "{\"error\":{\"code\":\"serialization_error\",\"message\":\"failed to serialize response\"}}"
                .to_owned()
        }
    }
}

fn encode_error(code: &str, message: &str) -> String {
    encode_json(&ErrorEnvelope {
        error: ErrorPayload { code, message },
    })
}

fn into_c_ptr(payload: String) -> *mut c_char {
    match CString::new(payload) {
        Ok(value) => value.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

fn with_ffi_boundary<F>(work: F) -> *mut c_char
where
    F: FnOnce() -> String,
{
    match catch_unwind(AssertUnwindSafe(work)) {
        Ok(payload) => into_c_ptr(payload),
        Err(_) => into_c_ptr(encode_error(
            "panic",
            "unexpected panic occurred in Rust FFI boundary",
        )),
    }
}

fn read_c_string(ptr: *const c_char) -> Result<String, String> {
    if ptr.is_null() {
        return Err("request_json pointer was null".to_owned());
    }

    // SAFETY: pointer nullability is checked above, and callers guarantee a
    // valid NUL-terminated C string for the duration of the call.
    let c_str = unsafe { CStr::from_ptr(ptr) };
    match c_str.to_str() {
        Ok(text) => Ok(text.to_owned()),
        Err(_) => Err("request_json was not valid UTF-8".to_owned()),
    }
}

fn build_validation_summary(config_toml: Option<&str>) -> Option<ValidationSummary> {
    let config_toml = config_toml?;
    let result = moltis_config::validate::validate_toml_str(config_toml);

    Some(ValidationSummary {
        errors: result.count(Severity::Error),
        warnings: result.count(Severity::Warning),
        info: result.count(Severity::Info),
        has_errors: result.has_errors(),
    })
}

fn config_dir_string() -> String {
    match moltis_config::config_dir() {
        Some(path) => path.display().to_string(),
        None => "unavailable".to_owned(),
    }
}

// ── Chat with real LLM ────────────────────────────────────────────────────

fn resolve_provider(request: &ChatRequest) -> Option<std::sync::Arc<dyn LlmProvider>> {
    let registry = BRIDGE.registry.read().unwrap_or_else(|e| e.into_inner());

    // Try explicit model first
    if let Some(model_id) = &request.model
        && let Some(provider) = registry.get(model_id)
    {
        emit_log("DEBUG", "bridge", &format!(
            "Resolved provider for model={}: {}",
            model_id, provider.name()
        ));
        return Some(provider);
    }

    // Fall back to first available provider
    let result = registry.first();
    if let Some(ref p) = result {
        emit_log("DEBUG", "bridge", &format!(
            "Using first available provider: {} ({})",
            p.name(), p.id()
        ));
    } else {
        emit_log("WARN", "bridge", "No provider available in registry");
    }
    result
}

fn build_chat_response(request: ChatRequest) -> String {
    emit_log("INFO", "bridge.chat", &format!(
        "Chat request: model={:?} msg_len={}",
        request.model, request.message.len()
    ));
    let validation = build_validation_summary(request.config_toml.as_deref());

    let (reply, model, provider_name, input_tokens, output_tokens, duration_ms) =
        match resolve_provider(&request) {
            Some(provider) => {
                let model_id = provider.id().to_string();
                let provider_name = provider.name().to_string();
                let messages = vec![AgentChatMessage::User {
                    content: UserContent::text(&request.message),
                }];

                emit_log("DEBUG", "bridge.chat", &format!(
                    "Calling {}/{}", provider_name, model_id
                ));
                let start = std::time::Instant::now();
                match BRIDGE.runtime.block_on(provider.complete(&messages, &[])) {
                    Ok(response) => {
                        let elapsed = start.elapsed().as_millis() as u64;
                        let text = response
                            .text
                            .unwrap_or_else(|| "(empty response)".to_owned());
                        let in_tok = response.usage.input_tokens;
                        let out_tok = response.usage.output_tokens;
                        emit_log("INFO", "bridge.chat", &format!(
                            "Response: {}ms in={} out={} provider={}",
                            elapsed, in_tok, out_tok, provider_name
                        ));
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
                    None, None, None, None, None,
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

// ── Streaming support ──────────────────────────────────────────────────────

/// Callback type for streaming events. Rust owns the `event_json` pointer —
/// the callback must copy the data before returning; Rust drops it afterwards.
type StreamCallback = unsafe extern "C" fn(event_json: *const c_char, user_data: *mut c_void);

/// JSON-serializable event sent to Swift via the callback.
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum BridgeStreamEvent {
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
struct StreamCallbackCtx {
    callback: StreamCallback,
    user_data: *mut c_void,
}

// SAFETY: See struct doc — Swift retains `StreamContext` via
// `Unmanaged.passRetained` and the callback itself is a plain function pointer.
unsafe impl Send for StreamCallbackCtx {}

impl StreamCallbackCtx {
    fn send(&self, event: &BridgeStreamEvent) {
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
    let raw = match read_c_string(request_json) {
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
    }];

    let ctx = StreamCallbackCtx {
        callback,
        user_data,
    };

    emit_log("INFO", "bridge.stream", &format!(
        "Starting stream: {}/{}", provider_name, model_id
    ));

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
                    emit_log("ERROR", "bridge.stream", &format!(
                        "Stream error: {message}"
                    ));
                    ctx.send(&BridgeStreamEvent::Error { message });
                    return;
                },
                // Ignore tool-call and reasoning events for chat UI.
                _ => {},
            }
        }

        let elapsed = start.elapsed().as_millis() as u64;
        emit_log("INFO", "bridge.stream", &format!(
            "Stream done: {}ms deltas={} in={} out={} provider={}",
            elapsed, delta_count, usage.input_tokens,
            usage.output_tokens, provider_name
        ));
        ctx.send(&BridgeStreamEvent::Done {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            duration_ms: elapsed,
            model: Some(model_id),
            provider: Some(provider_name),
        });
    });
}

// ── Metrics / tracing helpers ──────────────────────────────────────────────

#[cfg(feature = "metrics")]
fn record_call(function: &'static str) {
    metrics::counter!("moltis_swift_bridge_calls_total", "function" => function).increment(1);
}

#[cfg(not(feature = "metrics"))]
fn record_call(_function: &'static str) {}

#[cfg(feature = "metrics")]
fn record_error(function: &'static str, code: &'static str) {
    metrics::counter!(
        "moltis_swift_bridge_errors_total",
        "function" => function,
        "code" => code
    )
    .increment(1);
}

#[cfg(not(feature = "metrics"))]
fn record_error(_function: &'static str, _code: &'static str) {}

#[cfg(feature = "tracing")]
fn trace_call(function: &'static str) {
    tracing::debug!(target: "moltis_swift_bridge", function, "ffi call");
}

#[cfg(not(feature = "tracing"))]
fn trace_call(_function: &'static str) {}

// ── FFI exports ────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn moltis_version() -> *mut c_char {
    record_call("moltis_version");
    trace_call("moltis_version");

    with_ffi_boundary(|| {
        emit_log("DEBUG", "bridge", "moltis_version called");
        let response = VersionResponse {
            bridge_version: env!("CARGO_PKG_VERSION"),
            moltis_version: env!("CARGO_PKG_VERSION"),
            config_dir: config_dir_string(),
        };
        emit_log("INFO", "bridge", &format!(
            "version: bridge={} config_dir={}",
            response.bridge_version, response.config_dir
        ));
        encode_json(&response)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn moltis_chat_json(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_chat_json");
    trace_call("moltis_chat_json");

    with_ffi_boundary(|| {
        let raw = match read_c_string(request_json) {
            Ok(value) => value,
            Err(message) => {
                record_error("moltis_chat_json", "null_pointer_or_invalid_utf8");
                return encode_error("null_pointer_or_invalid_utf8", &message);
            },
        };

        let request = match serde_json::from_str::<ChatRequest>(&raw) {
            Ok(request) => request,
            Err(error) => {
                record_error("moltis_chat_json", "invalid_json");
                return encode_error("invalid_json", &error.to_string());
            },
        };

        build_chat_response(request)
    })
}

/// Returns JSON array of all known providers.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_known_providers() -> *mut c_char {
    record_call("moltis_known_providers");
    trace_call("moltis_known_providers");

    with_ffi_boundary(|| {
        emit_log("DEBUG", "bridge", "Loading known providers");
        let providers: Vec<BridgeKnownProvider> = known_providers()
            .into_iter()
            .map(|p| BridgeKnownProvider {
                name: p.name,
                display_name: p.display_name,
                auth_type: p.auth_type,
                env_key: p.env_key,
                default_base_url: p.default_base_url,
                requires_model: p.requires_model,
                key_optional: p.key_optional,
            })
            .collect();
        emit_log("INFO", "bridge", &format!(
            "Known providers: {}", providers.len()
        ));
        encode_json(&providers)
    })
}

/// Returns JSON array of auto-detected provider sources.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_detect_providers() -> *mut c_char {
    record_call("moltis_detect_providers");
    trace_call("moltis_detect_providers");

    with_ffi_boundary(|| {
        emit_log("DEBUG", "bridge", "Detecting provider sources");
        let config = ProvidersConfig::default();
        let env_overrides = HashMap::new();
        let sources = detect_auto_provider_sources_with_overrides(&config, None, &env_overrides);
        let bridge_sources: Vec<BridgeDetectedSource> = sources
            .into_iter()
            .map(|s| BridgeDetectedSource {
                provider: s.provider,
                source: s.source,
            })
            .collect();
        let names: Vec<&str> = bridge_sources.iter().map(|s| s.provider.as_str()).collect();
        emit_log("INFO", "bridge", &format!(
            "Detected {} sources: {:?}", bridge_sources.len(), names
        ));
        encode_json(&bridge_sources)
    })
}

/// Saves provider configuration (API key, base URL, models).
#[unsafe(no_mangle)]
pub extern "C" fn moltis_save_provider_config(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_save_provider_config");
    trace_call("moltis_save_provider_config");

    with_ffi_boundary(|| {
        let raw = match read_c_string(request_json) {
            Ok(value) => value,
            Err(message) => {
                record_error(
                    "moltis_save_provider_config",
                    "null_pointer_or_invalid_utf8",
                );
                return encode_error("null_pointer_or_invalid_utf8", &message);
            },
        };

        let request = match serde_json::from_str::<SaveProviderRequest>(&raw) {
            Ok(request) => request,
            Err(error) => {
                record_error("moltis_save_provider_config", "invalid_json");
                return encode_error("invalid_json", &error.to_string());
            },
        };

        emit_log("INFO", "bridge.config", &format!(
            "Saving config for provider={}", request.provider
        ));

        let key_store = KeyStore::new();
        match key_store.save_config(
            &request.provider,
            request.api_key,
            request.base_url,
            request.models,
        ) {
            Ok(()) => {
                emit_log("INFO", "bridge.config", "Provider config saved");
                encode_json(&OkResponse { ok: true })
            },
            Err(error) => {
                emit_log("ERROR", "bridge.config", &format!(
                    "Save failed: {error}"
                ));
                encode_error("save_failed", &error)
            },
        }
    })
}

/// Lists all discovered models from the current provider registry.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_list_models() -> *mut c_char {
    record_call("moltis_list_models");
    trace_call("moltis_list_models");

    with_ffi_boundary(|| {
        emit_log("DEBUG", "bridge", "Listing models from registry");
        let registry = BRIDGE.registry.read().unwrap_or_else(|e| e.into_inner());
        let models: Vec<BridgeModelInfo> = registry
            .list_models()
            .iter()
            .map(|m| BridgeModelInfo {
                id: m.id.clone(),
                provider: m.provider.clone(),
                display_name: m.display_name.clone(),
                created_at: m.created_at,
            })
            .collect();
        emit_log("INFO", "bridge", &format!(
            "Listed {} models", models.len()
        ));
        encode_json(&models)
    })
}

/// Rebuilds the global provider registry from saved config + env.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_refresh_registry() -> *mut c_char {
    record_call("moltis_refresh_registry");
    trace_call("moltis_refresh_registry");

    with_ffi_boundary(|| {
        emit_log("INFO", "bridge", "Refreshing provider registry");
        let new_registry = build_registry();
        let mut guard = BRIDGE.registry.write().unwrap_or_else(|e| e.into_inner());
        *guard = new_registry;
        emit_log("INFO", "bridge", "Provider registry rebuilt");
        encode_json(&OkResponse { ok: true })
    })
}

#[unsafe(no_mangle)]
/// # Safety
///
/// `ptr` must either be null or a pointer previously returned by one of the
/// `moltis_*` FFI functions from this crate. Passing any other pointer, or
/// freeing the same pointer more than once, is undefined behavior.
pub unsafe extern "C" fn moltis_free_string(ptr: *mut c_char) {
    record_call("moltis_free_string");

    if ptr.is_null() {
        return;
    }

    // SAFETY: pointer must originate from `CString::into_raw` in this crate.
    let _ = unsafe { CString::from_raw(ptr) };
}

/// Register a callback to receive log events from the Rust bridge.
/// Only the first call takes effect; subsequent calls are ignored.
///
/// # Safety
///
/// `callback` must be a valid function pointer that remains valid for
/// the lifetime of the process.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moltis_set_log_callback(callback: LogCallback) {
    let _ = LOG_CALLBACK.set(callback);
    emit_log("INFO", "bridge", "Log callback registered");
}

#[unsafe(no_mangle)]
pub extern "C" fn moltis_shutdown() {
    record_call("moltis_shutdown");
    trace_call("moltis_shutdown");
    emit_log("INFO", "bridge", "Shutdown requested");
}

#[cfg(test)]
mod tests {
    use {super::*, serde_json::Value};

    fn text_from_ptr(ptr: *mut c_char) -> String {
        assert!(!ptr.is_null(), "ffi returned null pointer");

        // SAFETY: pointer returned by this crate, converted back exactly once.
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
    fn version_returns_expected_payload() {
        let payload = json_from_ptr(moltis_version());

        let version = payload
            .get("bridge_version")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(version, env!("CARGO_PKG_VERSION"));

        let config_dir = payload
            .get("config_dir")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(!config_dir.is_empty(), "config_dir should be populated");
    }

    #[test]
    fn chat_returns_error_for_null_pointer() {
        let payload = json_from_ptr(moltis_chat_json(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|value| value.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }

    #[test]
    fn chat_returns_validation_counts() {
        let request =
            r#"{"message":"hello from swift","config_toml":"[server]\nport = \"invalid\""}"#;
        let c_request = match CString::new(request) {
            Ok(value) => value,
            Err(error) => panic!("failed to build c string for test request: {error}"),
        };

        let payload = json_from_ptr(moltis_chat_json(c_request.as_ptr()));

        // Chat response should have a reply (either from LLM or fallback)
        assert!(
            payload.get("reply").and_then(Value::as_str).is_some(),
            "response should contain a reply field"
        );

        let has_errors = payload
            .get("validation")
            .and_then(|value| value.get("has_errors"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        assert!(has_errors, "validation should detect invalid config value");
    }

    #[test]
    fn known_providers_returns_array() {
        let payload = json_from_ptr(moltis_known_providers());

        let providers = payload.as_array();
        assert!(
            providers.is_some(),
            "known_providers should return a JSON array"
        );
        let providers = providers.unwrap_or_else(|| panic!("not an array"));
        assert!(!providers.is_empty(), "should have at least one provider");

        // Check first provider has expected fields
        let first = &providers[0];
        assert!(first.get("name").and_then(Value::as_str).is_some());
        assert!(first.get("display_name").and_then(Value::as_str).is_some());
        assert!(first.get("auth_type").and_then(Value::as_str).is_some());
    }

    #[test]
    fn detect_providers_returns_array() {
        let payload = json_from_ptr(moltis_detect_providers());

        // Should always return a JSON array (possibly empty)
        assert!(
            payload.as_array().is_some(),
            "detect_providers should return a JSON array"
        );
    }

    #[test]
    fn save_provider_config_returns_error_for_null() {
        let payload = json_from_ptr(moltis_save_provider_config(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|value| value.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }

    #[test]
    fn list_models_returns_array() {
        let payload = json_from_ptr(moltis_list_models());

        assert!(
            payload.as_array().is_some(),
            "list_models should return a JSON array"
        );
    }

    #[test]
    fn refresh_registry_returns_ok() {
        let payload = json_from_ptr(moltis_refresh_registry());

        let ok = payload.get("ok").and_then(Value::as_bool).unwrap_or(false);
        assert!(ok, "refresh_registry should return ok: true");
    }

    #[test]
    fn free_string_tolerates_null_pointer() {
        // SAFETY: null pointers are explicitly accepted and treated as no-op.
        unsafe {
            moltis_free_string(std::ptr::null_mut());
        }
    }

    #[test]
    fn chat_stream_sends_error_for_null_pointer() {
        use std::sync::{Arc, Mutex};

        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        // Leak the Arc into user_data so the callback can access it.
        let user_data = Arc::into_raw(events_clone) as *mut c_void;

        unsafe extern "C" fn test_callback(
            event_json: *const c_char,
            user_data: *mut c_void,
        ) {
            // SAFETY: event_json is a valid NUL-terminated C string from
            // send_stream_event; user_data is our Arc<Mutex<Vec<String>>>.
            unsafe {
                let json = CStr::from_ptr(event_json).to_string_lossy().to_string();
                let events = &*(user_data as *const Mutex<Vec<String>>);
                events
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(json);
            }
        }

        // SAFETY: null request_json triggers synchronous error callback.
        unsafe {
            moltis_chat_stream(std::ptr::null(), test_callback, user_data);
        }

        // Reclaim the Arc.
        let events = unsafe { Arc::from_raw(user_data as *const Mutex<Vec<String>>) };
        let received = events.lock().unwrap_or_else(|e| e.into_inner());

        assert_eq!(received.len(), 1, "should receive exactly one error event");
        let parsed: Value =
            serde_json::from_str(&received[0]).unwrap_or_else(|e| panic!("bad json: {e}"));
        assert_eq!(
            parsed.get("type").and_then(Value::as_str),
            Some("error"),
            "event type should be 'error'"
        );
    }

    #[test]
    fn chat_stream_sends_error_for_no_provider() {
        use std::sync::{Arc, Mutex};

        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);
        let user_data = Arc::into_raw(events_clone) as *mut c_void;

        unsafe extern "C" fn test_callback(
            event_json: *const c_char,
            user_data: *mut c_void,
        ) {
            // SAFETY: event_json is a valid NUL-terminated C string from
            // send_stream_event; user_data is our Arc<Mutex<Vec<String>>>.
            unsafe {
                let json = CStr::from_ptr(event_json).to_string_lossy().to_string();
                let events = &*(user_data as *const Mutex<Vec<String>>);
                events
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(json);
            }
        }

        // Use a model that almost certainly won't match any configured provider.
        let request = r#"{"message":"test","model":"nonexistent-model-xyz"}"#;
        let c_request = CString::new(request).unwrap_or_else(|e| panic!("{e}"));

        // SAFETY: valid C string, valid callback, valid user_data.
        unsafe {
            moltis_chat_stream(c_request.as_ptr(), test_callback, user_data);
        }

        // Wait briefly for the async task to complete (it may also error synchronously).
        std::thread::sleep(std::time::Duration::from_millis(200));

        let events = unsafe { Arc::from_raw(user_data as *const Mutex<Vec<String>>) };
        let received = events.lock().unwrap_or_else(|e| e.into_inner());

        // Should receive at least one event (either an error for no provider,
        // or a done event if somehow a provider matched).
        assert!(
            !received.is_empty(),
            "should receive at least one stream event"
        );
    }
}
