//! C ABI bridge for embedding Moltis Rust functionality into native Swift apps.

#![allow(unsafe_code)]

use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_void},
    net::SocketAddr,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{LazyLock, Mutex, OnceLock, RwLock},
};

use {
    moltis_agents::model::{
        ChatMessage as AgentChatMessage, LlmProvider, StreamEvent, Usage, UserContent,
    },
    moltis_config::validate::Severity,
    moltis_provider_setup::{
        KeyStore, config_with_saved_keys, detect_auto_provider_sources_with_overrides,
        known_providers,
    },
    moltis_providers::ProviderRegistry,
    moltis_sessions::{
        message::PersistedMessage,
        metadata::{SessionEntry, SqliteSessionMetadata},
        session_events::{SessionEvent, SessionEventBus},
        store::SessionStore,
    },
    serde::{Deserialize, Serialize},
    tokio_stream::StreamExt,
};

// ── Global bridge state ────────────────────────────────────────────────────

struct BridgeState {
    runtime: tokio::runtime::Runtime,
    registry: RwLock<ProviderRegistry>,
    session_store: SessionStore,
    session_metadata: SqliteSessionMetadata,
}

impl BridgeState {
    fn new() -> Self {
        emit_log(
            "INFO",
            "bridge",
            "Initializing Rust bridge (tokio runtime + registry)",
        );
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap_or_else(|e| panic!("failed to create tokio runtime: {e}"));

        let registry = build_registry();

        // Initialize persistent session storage (JSONL message files).
        let data_dir = moltis_config::data_dir();
        let sessions_dir = data_dir.join("sessions");
        if let Err(e) = std::fs::create_dir_all(&sessions_dir) {
            emit_log(
                "ERROR",
                "bridge",
                &format!("Failed to create sessions dir: {e}"),
            );
        }
        let session_store = SessionStore::new(sessions_dir);

        // Open the shared SQLite database (same moltis.db used by the gateway).
        // WAL mode + synchronous=NORMAL avoids multi-second write contention.
        let db_path = data_dir.join("moltis.db");
        let db_pool = runtime.block_on(async {
            use {
                sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
                std::str::FromStr,
            };
            let opts = SqliteConnectOptions::from_str(&format!("sqlite:{}", db_path.display()))
                .expect("invalid moltis.db path")
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Normal);
            let pool = sqlx::SqlitePool::connect_with(opts)
                .await
                .unwrap_or_else(|e| panic!("failed to open moltis.db: {e}"));
            // Run migrations so the sessions table exists even if the gateway
            // hasn't been started yet. Order: projects first (FK dependency).
            if let Err(e) = moltis_projects::run_migrations(&pool).await {
                emit_log("WARN", "bridge", &format!("projects migration: {e}"));
            }
            if let Err(e) = moltis_sessions::run_migrations(&pool).await {
                emit_log("WARN", "bridge", &format!("sessions migration: {e}"));
            }
            pool
        });
        let event_bus = SessionEventBus::new();
        let session_metadata = SqliteSessionMetadata::with_event_bus(db_pool, event_bus);

        emit_log("INFO", "bridge", "Bridge initialized successfully");
        Self {
            runtime,
            registry: RwLock::new(registry),
            session_store,
            session_metadata,
        }
    }
}

fn build_registry() -> ProviderRegistry {
    let config = moltis_config::discover_and_load();
    let env_overrides = config.env.clone();
    let key_store = KeyStore::new();
    let effective = config_with_saved_keys(&config.providers, &key_store, &[]);
    ProviderRegistry::from_env_with_config_and_overrides(&effective, &env_overrides)
}

static BRIDGE: LazyLock<BridgeState> = LazyLock::new(BridgeState::new);

// ── HTTP Server ──────────────────────────────────────────────────────────

/// Handle to a running httpd server, used to shut it down.
struct HttpdHandle {
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
    addr: SocketAddr,
    /// Keep the gateway state alive while the server is running.
    _state: std::sync::Arc<moltis_gateway::state::GatewayState>,
}

/// Global server handle — `None` when stopped, `Some` when running.
static HTTPD: Mutex<Option<HttpdHandle>> = Mutex::new(None);

#[derive(Debug, Deserialize)]
struct StartHttpdRequest {
    #[serde(default = "default_httpd_host")]
    host: String,
    #[serde(default = "default_httpd_port")]
    port: u16,
    #[serde(default)]
    config_dir: Option<String>,
    #[serde(default)]
    data_dir: Option<String>,
}

fn default_httpd_host() -> String {
    "127.0.0.1".to_owned()
}

fn default_httpd_port() -> u16 {
    8080
}

#[derive(Debug, Serialize)]
struct HttpdStatusResponse {
    running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    addr: Option<String>,
}

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
/// Rust owns the `event_json` pointer — the callback must copy the data
/// before returning.
type SessionEventCallback = unsafe extern "C" fn(event_json: *const c_char);

static SESSION_EVENT_CALLBACK: OnceLock<SessionEventCallback> = OnceLock::new();

/// JSON payload sent to Swift for each session event.
#[derive(Debug, Serialize)]
struct BridgeSessionEvent {
    kind: &'static str,
    #[serde(rename = "sessionKey")]
    session_key: String,
}

fn emit_session_event(event: &SessionEvent) {
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
/// Rust owns the `event_json` pointer — the callback must copy the data
/// before returning.
type NetworkAuditCallback = unsafe extern "C" fn(event_json: *const c_char);

static NETWORK_AUDIT_CALLBACK: OnceLock<NetworkAuditCallback> = OnceLock::new();

/// JSON-serializable network audit event sent to Swift.
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

fn emit_network_audit(entry: &moltis_network_filter::NetworkAuditEntry) {
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

// ── Session types ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SwitchSessionRequest {
    key: String,
}

#[derive(Debug, Deserialize)]
struct CreateSessionRequest {
    #[serde(default)]
    label: Option<String>,
}

/// Compact session entry for the Swift side.
#[derive(Debug, Serialize)]
struct BridgeSessionEntry {
    key: String,
    label: Option<String>,
    message_count: u32,
    created_at: u64,
    updated_at: u64,
    preview: Option<String>,
}

impl From<&SessionEntry> for BridgeSessionEntry {
    fn from(e: &SessionEntry) -> Self {
        Self {
            key: e.key.clone(),
            label: e.label.clone(),
            message_count: e.message_count,
            created_at: e.created_at,
            updated_at: e.updated_at,
            preview: e.preview.clone(),
        }
    }
}

/// Session history: entry + messages.
#[derive(Debug, Serialize)]
struct BridgeSessionHistory {
    entry: BridgeSessionEntry,
    messages: Vec<serde_json::Value>,
}

/// Chat request with session key.
#[derive(Debug, Deserialize)]
struct SessionChatRequest {
    session_key: String,
    message: String,
    #[serde(default)]
    model: Option<String>,
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
    resolve_provider_for_model(request.model.as_deref())
}

fn resolve_provider_for_model(model: Option<&str>) -> Option<std::sync::Arc<dyn LlmProvider>> {
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

fn build_chat_response(request: ChatRequest) -> String {
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
        emit_log(
            "INFO",
            "bridge",
            &format!(
                "version: bridge={} config_dir={}",
                response.bridge_version, response.config_dir
            ),
        );
        encode_json(&response)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn moltis_get_identity() -> *mut c_char {
    record_call("moltis_get_identity");
    trace_call("moltis_get_identity");

    with_ffi_boundary(|| {
        let resolved = moltis_config::resolve_identity();
        emit_log("DEBUG", "bridge", "moltis_get_identity called");
        encode_json(&resolved)
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
                auth_type: p.auth_type.as_str(),
                env_key: p.env_key,
                default_base_url: p.default_base_url,
                requires_model: p.requires_model,
                key_optional: p.key_optional,
            })
            .collect();
        emit_log(
            "INFO",
            "bridge",
            &format!("Known providers: {}", providers.len()),
        );
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
        let config = moltis_config::discover_and_load();
        let sources =
            detect_auto_provider_sources_with_overrides(&config.providers, None, &config.env);
        let bridge_sources: Vec<BridgeDetectedSource> = sources
            .into_iter()
            .map(|s| BridgeDetectedSource {
                provider: s.provider,
                source: s.source,
            })
            .collect();
        let names: Vec<&str> = bridge_sources.iter().map(|s| s.provider.as_str()).collect();
        emit_log(
            "INFO",
            "bridge",
            &format!("Detected {} sources: {:?}", bridge_sources.len(), names),
        );
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

        emit_log(
            "INFO",
            "bridge.config",
            &format!("Saving config for provider={}", request.provider),
        );

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
                emit_log("ERROR", "bridge.config", &format!("Save failed: {error}"));
                encode_error("save_failed", &error.to_string())
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
        emit_log("INFO", "bridge", &format!("Listed {} models", models.len()));
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

/// Register a callback for session events (created, deleted, patched).
///
/// The callback receives a JSON string: `{"kind":"created","sessionKey":"..."}`.
/// Rust owns the pointer — the callback must copy the data before returning.
///
/// # Safety
///
/// `callback` must be a valid function pointer that remains valid for
/// the lifetime of the process.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moltis_set_session_event_callback(callback: SessionEventCallback) {
    if SESSION_EVENT_CALLBACK.set(callback).is_ok() {
        // Spawn a background task that subscribes to session events and
        // invokes the callback for each one.
        let bus = BRIDGE
            .session_metadata
            .event_bus()
            .expect("bridge session_metadata must have an event bus");
        let mut rx = bus.subscribe();
        BRIDGE.runtime.spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => emit_session_event(&event),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        emit_log(
                            "WARN",
                            "bridge.session_events",
                            &format!("Session event subscriber lagged, skipped {n} events"),
                        );
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        emit_log("INFO", "bridge", "Session event callback registered");
    }
}

/// Register a callback for network audit events (domain filter decisions).
///
/// The callback receives a JSON string with fields: `domain`, `port`,
/// `protocol`, `action`, `source`, and optionally `method` and `path`.
/// Rust owns the pointer — the callback must copy the data before returning.
///
/// # Safety
///
/// `callback` must be a valid function pointer that remains valid for
/// the lifetime of the process.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moltis_set_network_audit_callback(callback: NetworkAuditCallback) {
    let _ = NETWORK_AUDIT_CALLBACK.set(callback);
    emit_log("INFO", "bridge", "Network audit callback registered");
}

/// Starts the embedded HTTP server with the full Moltis gateway.
/// Returns JSON with `{"running": true, "addr": "..."}`.
/// If already running, returns the current status without restarting.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_start_httpd(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_start_httpd");
    trace_call("moltis_start_httpd");

    with_ffi_boundary(|| {
        let request: StartHttpdRequest = if request_json.is_null() {
            StartHttpdRequest {
                host: default_httpd_host(),
                port: default_httpd_port(),
                config_dir: None,
                data_dir: None,
            }
        } else {
            match read_c_string(request_json) {
                Ok(raw) => match serde_json::from_str(&raw) {
                    Ok(r) => r,
                    Err(e) => return encode_error("invalid_json", &e.to_string()),
                },
                Err(msg) => return encode_error("null_pointer_or_invalid_utf8", &msg),
            }
        };

        let mut guard = HTTPD.lock().unwrap_or_else(|e| e.into_inner());

        // Already running — return current status.
        if let Some(handle) = guard.as_ref() {
            emit_log(
                "INFO",
                "bridge.httpd",
                &format!("Server already running on {}", handle.addr),
            );
            return encode_json(&HttpdStatusResponse {
                running: true,
                addr: Some(handle.addr.to_string()),
            });
        }

        let bind_addr = format!("{}:{}", request.host, request.port);
        emit_log(
            "INFO",
            "bridge.httpd",
            &format!("Starting full gateway on {bind_addr}"),
        );

        // Prepare the full gateway (config, DB migrations, service wiring,
        // background tasks). This runs on the bridge runtime via block_on —
        // valid because this is an extern "C" fn, not async.
        let prepared =
            match BRIDGE
                .runtime
                .block_on(moltis_gateway::server::prepare_gateway_embedded(
                    &request.host,
                    request.port,
                    true, // no_tls — the macOS app manages its own TLS if needed
                    None, // log_buffer
                    request.config_dir.map(std::path::PathBuf::from),
                    request.data_dir.map(std::path::PathBuf::from),
                    Some(moltis_web::web_routes), // full web UI
                    BRIDGE.session_metadata.event_bus().cloned(), // share bus with gateway
                )) {
                Ok(p) => p,
                Err(e) => {
                    emit_log(
                        "ERROR",
                        "bridge.httpd",
                        &format!("Gateway init failed: {e}"),
                    );
                    return encode_error("gateway_init_failed", &e.to_string());
                },
            };

        let gateway_state = prepared.state;

        // Bind the TCP listener synchronously so we can report errors immediately.
        let listener = match BRIDGE
            .runtime
            .block_on(tokio::net::TcpListener::bind(&bind_addr))
        {
            Ok(l) => l,
            Err(e) => {
                emit_log("ERROR", "bridge.httpd", &format!("Bind failed: {e}"));
                return encode_error("bind_failed", &e.to_string());
            },
        };

        let addr = match listener.local_addr() {
            Ok(a) => a,
            Err(e) => return encode_error("addr_error", &e.to_string()),
        };

        // Subscribe to the network audit broadcast (if the proxy is active)
        // and forward entries to Swift via the registered callback.
        if let Some(ref audit_buf) = prepared.audit_buffer {
            let mut audit_rx = audit_buf.subscribe();
            BRIDGE.runtime.spawn(async move {
                loop {
                    match audit_rx.recv().await {
                        Ok(entry) => emit_network_audit(&entry),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });
            emit_log("INFO", "bridge.httpd", "Network audit bridge subscribed");
        }

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let app = prepared.app;

        BRIDGE.runtime.spawn(async move {
            let server = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            });
            if let Err(e) = server.await {
                emit_log("ERROR", "bridge.httpd", &format!("Server error: {e}"));
            }
            emit_log("INFO", "bridge.httpd", "Server stopped");
        });

        emit_log(
            "INFO",
            "bridge.httpd",
            &format!("Gateway listening on {addr}"),
        );
        *guard = Some(HttpdHandle {
            shutdown_tx,
            addr,
            _state: gateway_state,
        });

        encode_json(&HttpdStatusResponse {
            running: true,
            addr: Some(addr.to_string()),
        })
    })
}

/// Stops the embedded HTTP server. Returns `{"running": false}`.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_stop_httpd() -> *mut c_char {
    record_call("moltis_stop_httpd");
    trace_call("moltis_stop_httpd");

    with_ffi_boundary(|| {
        let mut guard = HTTPD.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(handle) = guard.take() {
            emit_log(
                "INFO",
                "bridge.httpd",
                &format!("Stopping httpd on {}", handle.addr),
            );
            let _ = handle.shutdown_tx.send(());
        } else {
            emit_log(
                "DEBUG",
                "bridge.httpd",
                "Stop called but server not running",
            );
        }
        encode_json(&HttpdStatusResponse {
            running: false,
            addr: None,
        })
    })
}

/// Returns the current httpd server status.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_httpd_status() -> *mut c_char {
    record_call("moltis_httpd_status");
    trace_call("moltis_httpd_status");

    with_ffi_boundary(|| {
        let guard = HTTPD.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(handle) => encode_json(&HttpdStatusResponse {
                running: true,
                addr: Some(handle.addr.to_string()),
            }),
            None => encode_json(&HttpdStatusResponse {
                running: false,
                addr: None,
            }),
        }
    })
}

// ── Session FFI exports ─────────────────────────────────────────────────

/// Returns JSON array of all session entries (sorted by created_at ASC, matching web UI).
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
#[unsafe(no_mangle)]
pub extern "C" fn moltis_switch_session(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_switch_session");
    trace_call("moltis_switch_session");

    with_ffi_boundary(|| {
        let raw = match read_c_string(request_json) {
            Ok(value) => value,
            Err(message) => return encode_error("null_pointer_or_invalid_utf8", &message),
        };

        let request = match serde_json::from_str::<SwitchSessionRequest>(&raw) {
            Ok(r) => r,
            Err(e) => return encode_error("invalid_json", &e.to_string()),
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

        unsafe extern "C" fn test_callback(event_json: *const c_char, user_data: *mut c_void) {
            // SAFETY: event_json is a valid NUL-terminated C string from
            // send_stream_event; user_data is our Arc<Mutex<Vec<String>>>.
            unsafe {
                let json = CStr::from_ptr(event_json).to_string_lossy().to_string();
                let events = &*(user_data as *const Mutex<Vec<String>>);
                events.lock().unwrap_or_else(|e| e.into_inner()).push(json);
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
    #[serial_test::serial]
    fn httpd_start_and_stop() {
        // Start on a random high port to avoid conflicts.
        let request = r#"{"host":"127.0.0.1","port":0}"#;
        let c_request = CString::new(request).unwrap_or_else(|e| panic!("{e}"));

        let payload = json_from_ptr(moltis_start_httpd(c_request.as_ptr()));
        assert_eq!(
            payload.get("running").and_then(Value::as_bool),
            Some(true),
            "server should be running after start"
        );
        assert!(
            payload.get("addr").and_then(Value::as_str).is_some(),
            "should report the bound address"
        );

        // Status should confirm running.
        let status = json_from_ptr(moltis_httpd_status());
        assert_eq!(status.get("running").and_then(Value::as_bool), Some(true),);

        // Stop.
        let stopped = json_from_ptr(moltis_stop_httpd());
        assert_eq!(stopped.get("running").and_then(Value::as_bool), Some(false),);

        // Status after stop.
        let status2 = json_from_ptr(moltis_httpd_status());
        assert_eq!(status2.get("running").and_then(Value::as_bool), Some(false),);
    }

    #[test]
    #[serial_test::serial]
    fn httpd_stop_when_not_running() {
        // Stop without start should still return running: false.
        let payload = json_from_ptr(moltis_stop_httpd());
        assert_eq!(payload.get("running").and_then(Value::as_bool), Some(false),);
    }

    #[test]
    fn chat_stream_sends_error_for_no_provider() {
        use std::sync::{Arc, Mutex};

        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);
        let user_data = Arc::into_raw(events_clone) as *mut c_void;

        unsafe extern "C" fn test_callback(event_json: *const c_char, user_data: *mut c_void) {
            // SAFETY: event_json is a valid NUL-terminated C string from
            // send_stream_event; user_data is our Arc<Mutex<Vec<String>>>.
            unsafe {
                let json = CStr::from_ptr(event_json).to_string_lossy().to_string();
                let events = &*(user_data as *const Mutex<Vec<String>>);
                events.lock().unwrap_or_else(|e| e.into_inner()).push(json);
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
