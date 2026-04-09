//! C ABI bridge for embedding Moltis Rust functionality into native Swift apps.

use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_void},
    net::SocketAddr,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, LazyLock, Mutex, OnceLock, RwLock},
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
    moltis_tools::image_cache::ImageBuilder,
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
    tokio_stream::StreamExt,
};

// ── Global bridge state ────────────────────────────────────────────────────

struct BridgeState {
    runtime: tokio::runtime::Runtime,
    registry: RwLock<ProviderRegistry>,
    session_store: SessionStore,
    session_metadata: SqliteSessionMetadata,
    credential_store: Arc<moltis_gateway::auth::CredentialStore>,
    sandbox_default_image_override: RwLock<Option<String>>,
}

impl BridgeState {
    fn new() -> Self {
        #[cfg(test)]
        init_swift_bridge_test_dirs();

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
            if let Err(e) = moltis_gateway::run_migrations(&pool).await {
                emit_log("WARN", "bridge", &format!("gateway migration: {e}"));
            }
            pool
        });
        let event_bus = SessionEventBus::new();
        let session_metadata = SqliteSessionMetadata::with_event_bus(db_pool.clone(), event_bus);
        let credential_store = runtime.block_on(async {
            // Keep vault metadata up to date so env var encryption status works
            // even when the full gateway server is not running.
            if let Err(e) = moltis_gateway::auth::moltis_vault::run_migrations(&db_pool).await {
                emit_log("WARN", "bridge", &format!("vault migration: {e}"));
            }

            let vault = match moltis_gateway::auth::moltis_vault::Vault::new(db_pool.clone()).await
            {
                Ok(vault) => Some(Arc::new(vault)),
                Err(e) => {
                    emit_log("WARN", "bridge", &format!("vault init failed: {e}"));
                    None
                },
            };

            match moltis_gateway::auth::CredentialStore::with_vault(
                db_pool.clone(),
                &moltis_config::discover_and_load().auth,
                vault,
            )
            .await
            {
                Ok(store) => Arc::new(store),
                Err(e) => panic!("failed to init credential store: {e}"),
            }
        });

        emit_log("INFO", "bridge", "Bridge initialized successfully");
        Self {
            runtime,
            registry: RwLock::new(registry),
            session_store,
            session_metadata,
            credential_store,
            sandbox_default_image_override: RwLock::new(None),
        }
    }
}

#[cfg(test)]
fn init_swift_bridge_test_dirs() {
    static TEST_DIRS_INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();

    TEST_DIRS_INIT.get_or_init(|| {
        let base = std::env::temp_dir().join(format!(
            "moltis-swift-bridge-tests-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let config_dir = base.join("config");
        let data_dir = base.join("data");

        if let Err(error) = std::fs::create_dir_all(&config_dir) {
            panic!("failed to create swift-bridge test config dir: {error}");
        }
        if let Err(error) = std::fs::create_dir_all(&data_dir) {
            panic!("failed to create swift-bridge test data dir: {error}");
        }

        moltis_config::set_config_dir(config_dir);
        moltis_config::set_data_dir(data_dir);
    });
}

fn build_registry() -> ProviderRegistry {
    let config = moltis_config::discover_and_load();
    let env_overrides = config.env.clone();
    let key_store = KeyStore::new();
    let effective = config_with_saved_keys(&config.providers, &key_store, &[]);
    #[cfg(test)]
    {
        ProviderRegistry::from_config_with_static_catalogs(&effective, &env_overrides)
    }
    #[cfg(not(test))]
    {
        ProviderRegistry::from_env_with_config_and_overrides(&effective, &env_overrides)
    }
}

static BRIDGE: LazyLock<BridgeState> = LazyLock::new(BridgeState::new);

// ── HTTP Server ──────────────────────────────────────────────────────────

/// Handle to a running httpd server, used to shut it down.
struct HttpdHandle {
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
    server_task: tokio::task::JoinHandle<()>,
    addr: SocketAddr,
    /// Gateway state — used for abort/peek FFI calls and kept alive while
    /// the server is running.
    state: std::sync::Arc<moltis_gateway::state::GatewayState>,
}

/// Global server handle — `None` when stopped, `Some` when running.
static HTTPD: Mutex<Option<HttpdHandle>> = Mutex::new(None);

fn stop_httpd_handle(handle: HttpdHandle, log_target: &str, stop_message: &str) {
    emit_log("INFO", log_target, stop_message);
    let _ = handle.shutdown_tx.send(());
    BRIDGE.runtime.block_on(async {
        if let Err(error) = handle.server_task.await {
            emit_log(
                "WARN",
                log_target,
                &format!("httpd task join failed during shutdown: {error}"),
            );
        }
    });
}

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
#[allow(unsafe_code)]
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

#[allow(unsafe_code)]
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
#[allow(unsafe_code)]
type SessionEventCallback = unsafe extern "C" fn(event_json: *const c_char);

static SESSION_EVENT_CALLBACK: OnceLock<SessionEventCallback> = OnceLock::new();

/// JSON payload sent to Swift for each session event.
#[derive(Debug, Serialize)]
struct BridgeSessionEvent {
    kind: &'static str,
    #[serde(rename = "sessionKey")]
    session_key: String,
}

#[allow(unsafe_code)]
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
#[allow(unsafe_code)]
type NetworkAuditCallback = unsafe extern "C" fn(event_json: *const c_char);

static NETWORK_AUDIT_CALLBACK: OnceLock<NetworkAuditCallback> = OnceLock::new();

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
    recommended: bool,
}

#[derive(Deserialize)]
struct SaveProviderRequest {
    provider: String,
    #[serde(default)]
    api_key: Option<Secret<String>>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    models: Option<Vec<String>>,
}

impl std::fmt::Debug for SaveProviderRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveProviderRequest")
            .field("provider", &self.provider)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("base_url", &self.base_url)
            .field("models", &self.models)
            .finish()
    }
}

#[derive(Debug, Serialize)]
struct OkResponse {
    ok: bool,
}

// ── Config / Identity / Soul request/response types ─────────────────────

#[derive(Debug, Serialize)]
struct GetConfigResponse {
    config: serde_json::Value,
    config_dir: String,
    data_dir: String,
}

#[derive(Debug, Serialize)]
struct GetSoulResponse {
    soul: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SaveSoulRequest {
    #[serde(default)]
    soul: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SaveIdentityRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    emoji: Option<String>,
    #[serde(default)]
    theme: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SaveUserProfileRequest {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SetEnvVarRequest {
    key: String,
    #[serde(default)]
    value: String,
}

#[derive(Debug, Deserialize)]
struct DeleteEnvVarRequest {
    id: i64,
}

#[derive(Debug, Serialize)]
struct ListEnvVarsResponse {
    env_vars: Vec<moltis_gateway::auth::EnvVarEntry>,
    vault_status: String,
}

#[derive(Debug, Serialize)]
struct MemoryStatusResponse {
    available: bool,
    total_files: usize,
    total_chunks: usize,
    db_size: u64,
    db_size_display: String,
    embedding_model: String,
    has_embeddings: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct MemoryConfigResponse {
    backend: String,
    citations: String,
    disable_rag: bool,
    llm_reranking: bool,
    session_export: bool,
    qmd_feature_enabled: bool,
}

#[derive(Debug, Deserialize)]
struct MemoryConfigUpdateRequest {
    #[serde(default)]
    backend: Option<String>,
    #[serde(default)]
    citations: Option<String>,
    #[serde(default)]
    llm_reranking: Option<bool>,
    #[serde(default)]
    disable_rag: Option<bool>,
    #[serde(default)]
    session_export: Option<bool>,
}

#[derive(Debug, Serialize)]
struct MemoryQmdStatusResponse {
    feature_enabled: bool,
    available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct AuthStatusResponse {
    auth_disabled: bool,
    has_password: bool,
    has_passkeys: bool,
    setup_complete: bool,
}

#[derive(Deserialize)]
struct AuthPasswordChangeRequest {
    #[serde(default)]
    current_password: Option<Secret<String>>,
    new_password: Secret<String>,
}

impl std::fmt::Debug for AuthPasswordChangeRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthPasswordChangeRequest")
            .field(
                "current_password",
                &self.current_password.as_ref().map(|_| "[REDACTED]"),
            )
            .field("new_password", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Serialize)]
struct AuthPasswordChangeResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery_key: Option<String>,
}

#[derive(Debug, Serialize)]
struct AuthPasskeysResponse {
    passkeys: Vec<moltis_gateway::auth::PasskeyEntry>,
}

#[derive(Debug, Deserialize)]
struct AuthPasskeyIdRequest {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct AuthPasskeyRenameRequest {
    id: i64,
    name: String,
}

const IMAGE_CACHE_DELETE_FAILED: &str = "IMAGE_CACHE_DELETE_FAILED";
const IMAGE_CACHE_PRUNE_FAILED: &str = "IMAGE_CACHE_PRUNE_FAILED";
const SANDBOX_CHECK_PACKAGES_FAILED: &str = "SANDBOX_CHECK_PACKAGES_FAILED";
const SANDBOX_BACKEND_UNAVAILABLE: &str = "SANDBOX_BACKEND_UNAVAILABLE";
const SANDBOX_IMAGE_NAME_REQUIRED: &str = "SANDBOX_IMAGE_NAME_REQUIRED";
const SANDBOX_IMAGE_PACKAGES_REQUIRED: &str = "SANDBOX_IMAGE_PACKAGES_REQUIRED";
const SANDBOX_IMAGE_NAME_INVALID: &str = "SANDBOX_IMAGE_NAME_INVALID";
const SANDBOX_TMP_DIR_CREATE_FAILED: &str = "SANDBOX_TMP_DIR_CREATE_FAILED";
const SANDBOX_DOCKERFILE_WRITE_FAILED: &str = "SANDBOX_DOCKERFILE_WRITE_FAILED";
const SANDBOX_IMAGE_BUILD_FAILED: &str = "SANDBOX_IMAGE_BUILD_FAILED";
const SANDBOX_CONTAINERS_LIST_FAILED: &str = "SANDBOX_CONTAINERS_LIST_FAILED";
const SANDBOX_CONTAINER_PREFIX_MISMATCH: &str = "SANDBOX_CONTAINER_PREFIX_MISMATCH";
const SANDBOX_CONTAINER_STOP_FAILED: &str = "SANDBOX_CONTAINER_STOP_FAILED";
const SANDBOX_CONTAINER_REMOVE_FAILED: &str = "SANDBOX_CONTAINER_REMOVE_FAILED";
const SANDBOX_CONTAINERS_CLEAN_FAILED: &str = "SANDBOX_CONTAINERS_CLEAN_FAILED";
const SANDBOX_DISK_USAGE_FAILED: &str = "SANDBOX_DISK_USAGE_FAILED";
const SANDBOX_DAEMON_RESTART_FAILED: &str = "SANDBOX_DAEMON_RESTART_FAILED";
const SANDBOX_SHARED_HOME_SAVE_FAILED: &str = "SANDBOX_SHARED_HOME_SAVE_FAILED";
const SANDBOX_PACKAGE_NAME_INVALID: &str = "SANDBOX_PACKAGE_NAME_INVALID";
const SANDBOX_BASE_IMAGE_INVALID: &str = "SANDBOX_BASE_IMAGE_INVALID";

/// Validates a package name to prevent shell injection.
/// Allows alphanumeric, hyphen, dot, plus, colon (covers dpkg naming conventions).
fn is_valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '+' | ':'))
}

/// Validates a container/base image reference (e.g. "ubuntu:25.10", "docker.io/library/ubuntu").
/// Allows alphanumeric, hyphen, dot, colon, slash, underscore.
fn is_valid_image_ref(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':' | '/' | '_'))
}

#[derive(Debug, Serialize)]
struct SandboxStatusResponse {
    backend: String,
    os: String,
    default_image: String,
}

#[derive(Debug, Serialize)]
struct SandboxImageEntry {
    tag: String,
    size: String,
    created: String,
    kind: String,
}

#[derive(Debug, Serialize)]
struct SandboxImagesResponse {
    images: Vec<SandboxImageEntry>,
}

#[derive(Debug, Deserialize)]
struct SandboxDeleteImageRequest {
    tag: String,
}

#[derive(Debug, Serialize)]
struct SandboxPruneImagesResponse {
    pruned: usize,
}

#[derive(Debug, Deserialize)]
struct SandboxCheckPackagesRequest {
    #[serde(default)]
    base: Option<String>,
    #[serde(default)]
    packages: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SandboxCheckPackagesResponse {
    found: HashMap<String, bool>,
}

#[derive(Debug, Deserialize)]
struct SandboxBuildImageRequest {
    #[serde(default)]
    name: String,
    #[serde(default)]
    base: Option<String>,
    #[serde(default)]
    packages: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SandboxBuildImageResponse {
    tag: String,
}

#[derive(Debug, Serialize)]
struct SandboxDefaultImageResponse {
    image: String,
}

#[derive(Debug, Deserialize)]
struct SandboxSetDefaultImageRequest {
    #[serde(default)]
    image: Option<String>,
}

#[derive(Debug, Serialize)]
struct SandboxSharedHomeConfigResponse {
    enabled: bool,
    mode: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    configured_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SandboxSharedHomeUpdateRequest {
    enabled: bool,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Serialize)]
struct SandboxSharedHomeSaveResponse {
    ok: bool,
    restart_required: bool,
    config_path: String,
    config: SandboxSharedHomeConfigResponse,
}

#[derive(Debug, Deserialize)]
struct SandboxContainerNameRequest {
    name: String,
}

#[derive(Debug, Serialize)]
struct SandboxContainersResponse {
    containers: Vec<moltis_tools::sandbox::RunningContainer>,
}

#[derive(Debug, Serialize)]
struct SandboxCleanContainersResponse {
    ok: bool,
    removed: usize,
}

#[derive(Debug, Serialize)]
struct SandboxDiskUsageResponse {
    usage: moltis_tools::sandbox::ContainerDiskUsage,
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

/// Parses a C string JSON pointer into a typed request, recording errors
/// against `function` for metrics. Returns `Err(encoded_error_json)` on
/// failure so callers can early-return from `with_ffi_boundary`.
fn parse_ffi_request<T: serde::de::DeserializeOwned>(
    function: &'static str,
    ptr: *const c_char,
) -> Result<T, String> {
    let raw = read_c_string(ptr).map_err(|message| {
        record_error(function, "null_pointer_or_invalid_utf8");
        encode_error("null_pointer_or_invalid_utf8", &message)
    })?;
    serde_json::from_str::<T>(&raw).map_err(|error| {
        record_error(function, "invalid_json");
        encode_error("invalid_json", &error.to_string())
    })
}

#[allow(unsafe_code)]
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

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    match bytes {
        b if b >= GB => format!("{:.1} GB", b as f64 / GB as f64),
        b if b >= MB => format!("{:.1} MB", b as f64 / MB as f64),
        b if b >= KB => format!("{:.1} KB", b as f64 / KB as f64),
        b => format!("{b} B"),
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

fn data_dir_string() -> String {
    moltis_config::data_dir().display().to_string()
}

fn vault_status_string() -> String {
    let Some(vault) = BRIDGE.credential_store.vault() else {
        return "disabled".to_owned();
    };
    match BRIDGE.runtime.block_on(async { vault.status().await }) {
        Ok(status) => format!("{status:?}").to_lowercase(),
        Err(_) => "error".to_owned(),
    }
}

fn sandbox_effective_default_image(config: &moltis_config::MoltisConfig) -> String {
    if let Some(value) = BRIDGE
        .sandbox_default_image_override
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
    {
        return value;
    }
    config
        .tools
        .exec
        .sandbox
        .image
        .clone()
        .unwrap_or_else(|| moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE.to_owned())
}

fn sandbox_backend_name(config: &moltis_config::MoltisConfig) -> String {
    let runtime_cfg = moltis_tools::sandbox::SandboxConfig::from(&config.tools.exec.sandbox);
    let backend = moltis_tools::sandbox::create_sandbox(runtime_cfg);
    backend.backend_name().to_owned()
}

fn sandbox_status_from_config(config: &moltis_config::MoltisConfig) -> SandboxStatusResponse {
    SandboxStatusResponse {
        backend: sandbox_backend_name(config),
        os: std::env::consts::OS.to_owned(),
        default_image: sandbox_effective_default_image(config),
    }
}

fn sandbox_container_prefix(config: &moltis_config::MoltisConfig) -> String {
    let runtime_cfg = moltis_tools::sandbox::SandboxConfig::from(&config.tools.exec.sandbox);
    runtime_cfg
        .container_prefix
        .unwrap_or_else(|| "moltis-sandbox".to_owned())
}

fn sandbox_shared_home_config_from_config(
    config: &moltis_config::MoltisConfig,
) -> SandboxSharedHomeConfigResponse {
    let runtime_cfg = moltis_tools::sandbox::SandboxConfig::from(&config.tools.exec.sandbox);
    let mode = match config.tools.exec.sandbox.home_persistence {
        moltis_config::schema::HomePersistenceConfig::Off => "off",
        moltis_config::schema::HomePersistenceConfig::Session => "session",
        moltis_config::schema::HomePersistenceConfig::Shared => "shared",
    };

    SandboxSharedHomeConfigResponse {
        enabled: matches!(
            config.tools.exec.sandbox.home_persistence,
            moltis_config::schema::HomePersistenceConfig::Shared
        ),
        mode: mode.to_owned(),
        path: moltis_tools::sandbox::shared_home_dir_path(&runtime_cfg)
            .display()
            .to_string(),
        configured_path: config.tools.exec.sandbox.shared_home_dir.clone(),
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
#[allow(unsafe_code)]
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
#[allow(unsafe_code)]
unsafe impl Send for StreamCallbackCtx {}

#[allow(unsafe_code)]
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

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_version() -> *mut c_char {
    record_call("moltis_version");
    trace_call("moltis_version");

    with_ffi_boundary(|| {
        emit_log("DEBUG", "bridge", "moltis_version called");
        let response = VersionResponse {
            bridge_version: moltis_config::VERSION,
            moltis_version: moltis_config::VERSION,
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

#[allow(unsafe_code)]
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

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_chat_json(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_chat_json");
    trace_call("moltis_chat_json");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<ChatRequest>("moltis_chat_json", request_json) {
            Ok(request) => request,
            Err(e) => return e,
        };

        build_chat_response(request)
    })
}

/// Returns JSON array of all known providers.
#[allow(unsafe_code)]
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
#[allow(unsafe_code)]
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
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_save_provider_config(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_save_provider_config");
    trace_call("moltis_save_provider_config");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SaveProviderRequest>(
            "moltis_save_provider_config",
            request_json,
        ) {
            Ok(request) => request,
            Err(e) => return e,
        };

        emit_log(
            "INFO",
            "bridge.config",
            &format!("Saving config for provider={}", request.provider),
        );

        let key_store = KeyStore::new();
        let api_key = request.api_key.map(|s| s.expose_secret().clone());
        match key_store.save_config(&request.provider, api_key, request.base_url, request.models) {
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
#[allow(unsafe_code)]
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
                recommended: m.recommended,
            })
            .collect();
        emit_log("INFO", "bridge", &format!("Listed {} models", models.len()));
        encode_json(&models)
    })
}

/// Rebuilds the global provider registry from saved config + env.
#[allow(unsafe_code)]
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

#[allow(unsafe_code)]
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
#[allow(unsafe_code)]
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
#[allow(unsafe_code)]
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
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moltis_set_network_audit_callback(callback: NetworkAuditCallback) {
    let _ = NETWORK_AUDIT_CALLBACK.set(callback);
    emit_log("INFO", "bridge", "Network audit callback registered");
}

/// Starts the embedded HTTP server with the full Moltis gateway.
/// Returns JSON with `{"running": true, "addr": "..."}`.
/// If already running, returns the current status without restarting.
#[allow(unsafe_code)]
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
        let prepared = match BRIDGE
            .runtime
            .block_on(moltis_httpd::prepare_httpd_embedded(
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
        #[cfg(feature = "trusted-network")]
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
        // Keep the proxy shutdown sender alive for the server's full lifetime;
        // dropping it closes the watch channel and terminates the proxy.
        #[cfg(feature = "trusted-network")]
        let _proxy_shutdown_tx = prepared._proxy_shutdown_tx;

        let server_task = BRIDGE.runtime.spawn(async move {
            // Hold the proxy sender inside the spawn so it lives as long as the server.
            #[cfg(feature = "trusted-network")]
            let _keep_proxy = _proxy_shutdown_tx;
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
            server_task,
            addr,
            state: gateway_state,
        });

        encode_json(&HttpdStatusResponse {
            running: true,
            addr: Some(addr.to_string()),
        })
    })
}

/// Stops the embedded HTTP server. Returns `{"running": false}`.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_stop_httpd() -> *mut c_char {
    record_call("moltis_stop_httpd");
    trace_call("moltis_stop_httpd");

    with_ffi_boundary(|| {
        let mut guard = HTTPD.lock().unwrap_or_else(|e| e.into_inner());
        let handle = guard.take();
        drop(guard);
        if let Some(handle) = handle {
            let message = format!("Stopping httpd on {}", handle.addr);
            stop_httpd_handle(handle, "bridge.httpd", &message);
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
#[allow(unsafe_code)]
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

// ── Abort / Peek FFI ────────────────────────────────────────────────────

/// Abort the active generation for a session. Requires the gateway to be
/// running (via `moltis_start_httpd`). Returns JSON with `{"aborted": bool}`.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_abort_session(session_key: *const c_char) -> *mut c_char {
    record_call("moltis_abort_session");
    trace_call("moltis_abort_session");

    with_ffi_boundary(|| {
        let key = match read_c_string(session_key) {
            Ok(k) => k,
            Err(msg) => return encode_error("invalid_session_key", &msg),
        };
        let guard = HTTPD.lock().unwrap_or_else(|e| e.into_inner());
        let Some(handle) = guard.as_ref() else {
            return encode_error("gateway_not_running", "start the gateway first");
        };
        let state = std::sync::Arc::clone(&handle.state);
        drop(guard);

        let params = serde_json::json!({ "sessionKey": key });
        match BRIDGE
            .runtime
            .block_on(async { state.chat().await.abort(params).await })
        {
            Ok(res) => encode_json(&res),
            Err(e) => encode_error("abort_failed", &e.to_string()),
        }
    })
}

/// Peek at the current activity for a session. Requires the gateway to be
/// running. Returns JSON with `{"active": bool, ...}`.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_peek_session(session_key: *const c_char) -> *mut c_char {
    record_call("moltis_peek_session");
    trace_call("moltis_peek_session");

    with_ffi_boundary(|| {
        let key = match read_c_string(session_key) {
            Ok(k) => k,
            Err(msg) => return encode_error("invalid_session_key", &msg),
        };
        let guard = HTTPD.lock().unwrap_or_else(|e| e.into_inner());
        let Some(handle) = guard.as_ref() else {
            return encode_error("gateway_not_running", "start the gateway first");
        };
        let state = std::sync::Arc::clone(&handle.state);
        drop(guard);

        let params = serde_json::json!({ "sessionKey": key });
        match BRIDGE
            .runtime
            .block_on(async { state.chat().await.peek(params).await })
        {
            Ok(res) => encode_json(&res),
            Err(e) => encode_error("peek_failed", &e.to_string()),
        }
    })
}

// ── Session FFI exports ─────────────────────────────────────────────────

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

// ── Config / Identity / Soul FFI ─────────────────────────────────────────

/// Returns the full `MoltisConfig` as JSON together with `config_dir` and
/// `data_dir` paths. Swift uses this to populate all settings panels.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_get_config() -> *mut c_char {
    record_call("moltis_get_config");
    trace_call("moltis_get_config");

    with_ffi_boundary(|| {
        emit_log("DEBUG", "bridge", "moltis_get_config called");
        let config = moltis_config::discover_and_load();
        let config_value = match serde_json::to_value(&config) {
            Ok(v) => v,
            Err(e) => return encode_error("serialization_error", &e.to_string()),
        };
        let response = GetConfigResponse {
            config: config_value,
            config_dir: config_dir_string(),
            data_dir: data_dir_string(),
        };
        emit_log("INFO", "bridge", "Config loaded for settings");
        encode_json(&response)
    })
}

/// Accepts a full `MoltisConfig` JSON and saves it via `save_config()`.
/// The TOML writer preserves existing comments in the config file.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_save_config(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_save_config");
    trace_call("moltis_save_config");

    with_ffi_boundary(|| {
        let config = match parse_ffi_request::<moltis_config::MoltisConfig>(
            "moltis_save_config",
            request_json,
        ) {
            Ok(c) => c,
            Err(e) => return e,
        };

        emit_log("INFO", "bridge.config", "Saving full config from settings");
        match moltis_config::save_config(&config) {
            Ok(path) => {
                emit_log(
                    "INFO",
                    "bridge.config",
                    &format!("Config saved to {}", path.display()),
                );
                encode_json(&OkResponse { ok: true })
            },
            Err(e) => {
                emit_log("ERROR", "bridge.config", &format!("Save failed: {e}"));
                encode_error("save_failed", &e.to_string())
            },
        }
    })
}

/// Returns memory status (counts + db size) for the settings panel.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_memory_status() -> *mut c_char {
    record_call("moltis_memory_status");
    trace_call("moltis_memory_status");

    with_ffi_boundary(|| {
        use {sqlx::sqlite::SqliteConnectOptions, std::str::FromStr};

        let config = moltis_config::discover_and_load();
        let embedding_model = config
            .memory
            .model
            .clone()
            .unwrap_or_else(|| "none".to_owned());
        let has_embeddings = !config.memory.disable_rag;

        let db_path = moltis_config::data_dir().join("memory.db");
        let db_size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);

        if !db_path.exists() {
            let response = MemoryStatusResponse {
                available: false,
                total_files: 0,
                total_chunks: 0,
                db_size,
                db_size_display: format_bytes(db_size),
                embedding_model,
                has_embeddings,
                error: Some("memory.db not found".to_owned()),
            };
            return encode_json(&response);
        }

        let options = match SqliteConnectOptions::from_str(&format!("sqlite:{}", db_path.display()))
        {
            Ok(opts) => opts.create_if_missing(false).read_only(true),
            Err(error) => {
                let response = MemoryStatusResponse {
                    available: false,
                    total_files: 0,
                    total_chunks: 0,
                    db_size,
                    db_size_display: format_bytes(db_size),
                    embedding_model,
                    has_embeddings,
                    error: Some(format!("invalid sqlite path: {error}")),
                };
                return encode_json(&response);
            },
        };

        let pool = match BRIDGE
            .runtime
            .block_on(sqlx::SqlitePool::connect_with(options))
        {
            Ok(pool) => pool,
            Err(error) => {
                let response = MemoryStatusResponse {
                    available: false,
                    total_files: 0,
                    total_chunks: 0,
                    db_size,
                    db_size_display: format_bytes(db_size),
                    embedding_model,
                    has_embeddings,
                    error: Some(format!("failed to open memory.db: {error}")),
                };
                return encode_json(&response);
            },
        };

        let (total_files, total_chunks) = BRIDGE.runtime.block_on(async {
            let has_files_table: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'files'",
            )
            .fetch_one(&pool)
            .await
            .unwrap_or(0);
            let has_chunks_table: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'chunks'",
            )
            .fetch_one(&pool)
            .await
            .unwrap_or(0);

            let files: i64 = if has_files_table > 0 {
                sqlx::query_scalar("SELECT COUNT(*) FROM files")
                    .fetch_one(&pool)
                    .await
                    .unwrap_or(0)
            } else {
                0
            };
            let chunks: i64 = if has_chunks_table > 0 {
                sqlx::query_scalar("SELECT COUNT(*) FROM chunks")
                    .fetch_one(&pool)
                    .await
                    .unwrap_or(0)
            } else {
                0
            };

            let files_count: usize = files.max(0).try_into().unwrap_or(0);
            let chunk_count: usize = chunks.max(0).try_into().unwrap_or(0);
            (files_count, chunk_count)
        });
        BRIDGE.runtime.block_on(pool.close());

        let response = MemoryStatusResponse {
            available: true,
            total_files,
            total_chunks,
            db_size,
            db_size_display: format_bytes(db_size),
            embedding_model,
            has_embeddings,
            error: None,
        };
        encode_json(&response)
    })
}

/// Returns memory configuration fields used by the settings panel.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_memory_config_get() -> *mut c_char {
    record_call("moltis_memory_config_get");
    trace_call("moltis_memory_config_get");

    with_ffi_boundary(|| {
        let config = moltis_config::discover_and_load();
        let memory = config.memory;
        let response = MemoryConfigResponse {
            backend: memory.backend.unwrap_or_else(|| "builtin".to_owned()),
            citations: memory.citations.unwrap_or_else(|| "auto".to_owned()),
            disable_rag: memory.disable_rag,
            llm_reranking: memory.llm_reranking,
            session_export: memory.session_export,
            qmd_feature_enabled: cfg!(feature = "qmd"),
        };
        encode_json(&response)
    })
}

/// Updates memory configuration fields used by the settings panel.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_memory_config_update(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_memory_config_update");
    trace_call("moltis_memory_config_update");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<MemoryConfigUpdateRequest>(
            "moltis_memory_config_update",
            request_json,
        ) {
            Ok(request) => request,
            Err(e) => return e,
        };

        let current = moltis_config::discover_and_load().memory;
        let backend = request
            .backend
            .unwrap_or_else(|| current.backend.unwrap_or_else(|| "builtin".to_owned()));
        let citations = request
            .citations
            .unwrap_or_else(|| current.citations.unwrap_or_else(|| "auto".to_owned()));
        let llm_reranking = request.llm_reranking.unwrap_or(current.llm_reranking);
        let session_export = request.session_export.unwrap_or(current.session_export);
        let mut disable_rag = current.disable_rag;

        let backend_value = backend.clone();
        let citations_value = citations.clone();

        if let Err(error) = moltis_config::update_config(|cfg| {
            cfg.memory.backend = Some(backend_value.clone());
            cfg.memory.citations = Some(citations_value.clone());
            cfg.memory.llm_reranking = llm_reranking;
            if let Some(value) = request.disable_rag {
                cfg.memory.disable_rag = value;
            }
            cfg.memory.session_export = session_export;
            disable_rag = cfg.memory.disable_rag;
        }) {
            record_error("moltis_memory_config_update", "save_failed");
            return encode_error("save_failed", &error.to_string());
        }

        let response = MemoryConfigResponse {
            backend,
            citations,
            disable_rag,
            llm_reranking,
            session_export,
            qmd_feature_enabled: cfg!(feature = "qmd"),
        };
        encode_json(&response)
    })
}

/// Returns QMD availability (binary detection + optional version).
#[unsafe(no_mangle)]
pub extern "C" fn moltis_memory_qmd_status() -> *mut c_char {
    record_call("moltis_memory_qmd_status");
    trace_call("moltis_memory_qmd_status");

    with_ffi_boundary(|| {
        if !cfg!(feature = "qmd") {
            let response = MemoryQmdStatusResponse {
                feature_enabled: false,
                available: false,
                version: None,
                error: Some("QMD feature is disabled in this build".to_owned()),
            };
            return encode_json(&response);
        }

        let command = moltis_config::discover_and_load()
            .memory
            .qmd
            .command
            .unwrap_or_else(|| "qmd".to_owned());

        let output = std::process::Command::new(&command)
            .arg("--version")
            .output();

        let response = match output {
            Ok(out) if out.status.success() => {
                let version = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                let resolved_version = if version.is_empty() {
                    None
                } else {
                    Some(version)
                };
                MemoryQmdStatusResponse {
                    feature_enabled: true,
                    available: true,
                    version: resolved_version,
                    error: None,
                }
            },
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
                let detail = if stderr.is_empty() {
                    format!("{command} --version exited with status {}", out.status)
                } else {
                    stderr
                };
                MemoryQmdStatusResponse {
                    feature_enabled: true,
                    available: false,
                    version: None,
                    error: Some(detail),
                }
            },
            Err(error) => MemoryQmdStatusResponse {
                feature_enabled: true,
                available: false,
                version: None,
                error: Some(error.to_string()),
            },
        };

        encode_json(&response)
    })
}

/// Returns the soul text from `SOUL.md`.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_get_soul() -> *mut c_char {
    record_call("moltis_get_soul");
    trace_call("moltis_get_soul");

    with_ffi_boundary(|| {
        emit_log("DEBUG", "bridge", "moltis_get_soul called");
        let soul = moltis_config::load_soul_for_agent("main");
        encode_json(&GetSoulResponse { soul })
    })
}

/// Saves soul text to `SOUL.md`. Pass `{"soul": null}` to clear.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_save_soul(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_save_soul");
    trace_call("moltis_save_soul");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SaveSoulRequest>("moltis_save_soul", request_json) {
            Ok(r) => r,
            Err(e) => return e,
        };

        emit_log("INFO", "bridge.config", "Saving soul from settings");
        match moltis_config::save_soul_for_agent("main", request.soul.as_deref()) {
            Ok(path) => {
                emit_log(
                    "INFO",
                    "bridge.config",
                    &format!("Soul saved to {}", path.display()),
                );
                encode_json(&OkResponse { ok: true })
            },
            Err(e) => {
                emit_log("ERROR", "bridge.config", &format!("Soul save failed: {e}"));
                encode_error("save_failed", &e.to_string())
            },
        }
    })
}

/// Saves identity (name, emoji, theme) to `IDENTITY.md`.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_save_identity(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_save_identity");
    trace_call("moltis_save_identity");

    with_ffi_boundary(|| {
        let request =
            match parse_ffi_request::<SaveIdentityRequest>("moltis_save_identity", request_json) {
                Ok(r) => r,
                Err(e) => return e,
            };

        let identity = moltis_config::AgentIdentity {
            name: request.name,
            emoji: request.emoji,
            theme: request.theme,
        };

        emit_log("INFO", "bridge.config", "Saving identity from settings");
        match moltis_config::save_identity_for_agent("main", &identity) {
            Ok(path) => {
                emit_log(
                    "INFO",
                    "bridge.config",
                    &format!("Identity saved to {}", path.display()),
                );
                encode_json(&OkResponse { ok: true })
            },
            Err(e) => {
                emit_log(
                    "ERROR",
                    "bridge.config",
                    &format!("Identity save failed: {e}"),
                );
                encode_error("save_failed", &e.to_string())
            },
        }
    })
}

/// Saves user profile (name) to `USER.md`.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_save_user_profile(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_save_user_profile");
    trace_call("moltis_save_user_profile");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SaveUserProfileRequest>(
            "moltis_save_user_profile",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let user = moltis_config::UserProfile {
            name: request.name,
            ..Default::default()
        };

        emit_log("INFO", "bridge.config", "Saving user profile from settings");
        match moltis_config::save_user(&user) {
            Ok(path) => {
                emit_log(
                    "INFO",
                    "bridge.config",
                    &format!("User profile saved to {}", path.display()),
                );
                encode_json(&OkResponse { ok: true })
            },
            Err(e) => {
                emit_log(
                    "ERROR",
                    "bridge.config",
                    &format!("User profile save failed: {e}"),
                );
                encode_error("save_failed", &e.to_string())
            },
        }
    })
}

/// Returns runtime environment variables from the credential store.
/// Values are never returned, only metadata (id/key/timestamps/encrypted).
#[unsafe(no_mangle)]
pub extern "C" fn moltis_list_env_vars() -> *mut c_char {
    record_call("moltis_list_env_vars");
    trace_call("moltis_list_env_vars");

    with_ffi_boundary(|| {
        let env_vars = match BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.list_env_vars())
        {
            Ok(vars) => vars,
            Err(e) => {
                record_error("moltis_list_env_vars", "ENV_LIST_FAILED");
                return encode_error("ENV_LIST_FAILED", &e.to_string());
            },
        };

        encode_json(&ListEnvVarsResponse {
            env_vars,
            vault_status: vault_status_string(),
        })
    })
}

/// Set (upsert) an environment variable in the credential store.
/// Uses vault encryption automatically when the vault is unsealed.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_set_env_var(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_set_env_var");
    trace_call("moltis_set_env_var");

    with_ffi_boundary(|| {
        let request =
            match parse_ffi_request::<SetEnvVarRequest>("moltis_set_env_var", request_json) {
                Ok(r) => r,
                Err(e) => return e,
            };

        let key = request.key.trim();
        if key.is_empty() {
            record_error("moltis_set_env_var", "ENV_KEY_REQUIRED");
            return encode_error("ENV_KEY_REQUIRED", "key is required");
        }
        if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            record_error("moltis_set_env_var", "ENV_KEY_INVALID");
            return encode_error(
                "ENV_KEY_INVALID",
                "key must contain only letters, digits, and underscores",
            );
        }

        match BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.set_env_var(key, &request.value))
        {
            Ok(_) => encode_json(&OkResponse { ok: true }),
            Err(e) => {
                record_error("moltis_set_env_var", "ENV_SET_FAILED");
                encode_error("ENV_SET_FAILED", &e.to_string())
            },
        }
    })
}

/// Delete an environment variable by ID.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_delete_env_var(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_delete_env_var");
    trace_call("moltis_delete_env_var");

    with_ffi_boundary(|| {
        let request =
            match parse_ffi_request::<DeleteEnvVarRequest>("moltis_delete_env_var", request_json) {
                Ok(r) => r,
                Err(e) => return e,
            };

        match BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.delete_env_var(request.id))
        {
            Ok(_) => encode_json(&OkResponse { ok: true }),
            Err(e) => {
                record_error("moltis_delete_env_var", "ENV_DELETE_FAILED");
                encode_error("ENV_DELETE_FAILED", &e.to_string())
            },
        }
    })
}

/// Returns authentication status for the HTTP server.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_auth_status() -> *mut c_char {
    record_call("moltis_auth_status");
    trace_call("moltis_auth_status");

    with_ffi_boundary(|| {
        let has_password = match BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.has_password())
        {
            Ok(value) => value,
            Err(error) => {
                record_error("moltis_auth_status", "AUTH_STATUS_FAILED");
                return encode_error("AUTH_STATUS_FAILED", &error.to_string());
            },
        };

        let has_passkeys = match BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.has_passkeys())
        {
            Ok(value) => value,
            Err(error) => {
                record_error("moltis_auth_status", "AUTH_STATUS_FAILED");
                return encode_error("AUTH_STATUS_FAILED", &error.to_string());
            },
        };

        encode_json(&AuthStatusResponse {
            auth_disabled: BRIDGE.credential_store.is_auth_disabled(),
            has_password,
            has_passkeys,
            setup_complete: BRIDGE.credential_store.is_setup_complete(),
        })
    })
}

/// Adds or changes the authentication password.
///
/// Accepts JSON:
/// - `{"new_password":"..."}` to set the first password.
/// - `{"current_password":"...","new_password":"..."}` to rotate.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_auth_password_change(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_auth_password_change");
    trace_call("moltis_auth_password_change");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<AuthPasswordChangeRequest>(
            "moltis_auth_password_change",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        if request.new_password.expose_secret().len() < 8 {
            record_error("moltis_auth_password_change", "AUTH_PASSWORD_TOO_SHORT");
            return encode_error(
                "AUTH_PASSWORD_TOO_SHORT",
                "new password must be at least 8 characters",
            );
        }

        let has_password = match BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.has_password())
        {
            Ok(value) => value,
            Err(error) => {
                record_error("moltis_auth_password_change", "AUTH_STATUS_FAILED");
                return encode_error("AUTH_STATUS_FAILED", &error.to_string());
            },
        };

        let mut recovery_key: Option<String> = None;

        let new_password = request.new_password.expose_secret();

        if has_password {
            let current_password = request
                .current_password
                .as_ref()
                .map(|s| s.expose_secret().as_str())
                .unwrap_or("");
            if let Err(error) = BRIDGE.runtime.block_on(
                BRIDGE
                    .credential_store
                    .change_password(current_password, new_password),
            ) {
                let message = error.to_string();
                if message.contains("incorrect") {
                    record_error(
                        "moltis_auth_password_change",
                        "AUTH_INVALID_CURRENT_PASSWORD",
                    );
                    return encode_error("AUTH_INVALID_CURRENT_PASSWORD", &message);
                }
                record_error("moltis_auth_password_change", "AUTH_PASSWORD_CHANGE_FAILED");
                return encode_error("AUTH_PASSWORD_CHANGE_FAILED", &message);
            }

            if let Some(vault) = BRIDGE.credential_store.vault()
                && let Err(error) = BRIDGE
                    .runtime
                    .block_on(vault.change_password(current_password, new_password))
            {
                emit_log(
                    "WARN",
                    "bridge.auth",
                    &format!("Vault password rotation failed: {error}"),
                );
            }
        } else if let Err(error) = BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.add_password(new_password))
        {
            record_error("moltis_auth_password_change", "AUTH_PASSWORD_SET_FAILED");
            return encode_error("AUTH_PASSWORD_SET_FAILED", &error.to_string());
        } else if let Some(vault) = BRIDGE.credential_store.vault() {
            match BRIDGE.runtime.block_on(vault.initialize(new_password)) {
                Ok(key) => {
                    recovery_key = Some(key.phrase().to_owned());
                },
                Err(moltis_gateway::auth::moltis_vault::VaultError::AlreadyInitialized) => {
                    if let Err(error) = BRIDGE.runtime.block_on(vault.unseal(new_password)) {
                        emit_log(
                            "WARN",
                            "bridge.auth",
                            &format!("Vault unseal failed after password set: {error}"),
                        );
                    }
                },
                Err(error) => {
                    emit_log(
                        "WARN",
                        "bridge.auth",
                        &format!("Vault initialization failed after password set: {error}"),
                    );
                },
            }
        }

        encode_json(&AuthPasswordChangeResponse {
            ok: true,
            recovery_key,
        })
    })
}

/// Removes all authentication credentials (passwords, passkeys, sessions, API keys).
#[unsafe(no_mangle)]
pub extern "C" fn moltis_auth_reset() -> *mut c_char {
    record_call("moltis_auth_reset");
    trace_call("moltis_auth_reset");

    with_ffi_boundary(
        || match BRIDGE.runtime.block_on(BRIDGE.credential_store.reset_all()) {
            Ok(()) => encode_json(&OkResponse { ok: true }),
            Err(error) => {
                record_error("moltis_auth_reset", "AUTH_RESET_FAILED");
                encode_error("AUTH_RESET_FAILED", &error.to_string())
            },
        },
    )
}

/// Lists all registered passkeys.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_auth_list_passkeys() -> *mut c_char {
    record_call("moltis_auth_list_passkeys");
    trace_call("moltis_auth_list_passkeys");

    with_ffi_boundary(|| {
        let passkeys = match BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.list_passkeys())
        {
            Ok(entries) => entries,
            Err(error) => {
                record_error("moltis_auth_list_passkeys", "AUTH_PASSKEY_LIST_FAILED");
                return encode_error("AUTH_PASSKEY_LIST_FAILED", &error.to_string());
            },
        };

        encode_json(&AuthPasskeysResponse { passkeys })
    })
}

/// Removes a passkey by database ID.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_auth_remove_passkey(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_auth_remove_passkey");
    trace_call("moltis_auth_remove_passkey");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<AuthPasskeyIdRequest>(
            "moltis_auth_remove_passkey",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        match BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.remove_passkey(request.id))
        {
            Ok(()) => encode_json(&OkResponse { ok: true }),
            Err(error) => {
                record_error("moltis_auth_remove_passkey", "AUTH_PASSKEY_REMOVE_FAILED");
                encode_error("AUTH_PASSKEY_REMOVE_FAILED", &error.to_string())
            },
        }
    })
}

/// Renames a passkey.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_auth_rename_passkey(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_auth_rename_passkey");
    trace_call("moltis_auth_rename_passkey");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<AuthPasskeyRenameRequest>(
            "moltis_auth_rename_passkey",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let name = request.name.trim();
        if name.is_empty() {
            record_error("moltis_auth_rename_passkey", "AUTH_PASSKEY_NAME_REQUIRED");
            return encode_error("AUTH_PASSKEY_NAME_REQUIRED", "name cannot be empty");
        }

        match BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.rename_passkey(request.id, name))
        {
            Ok(()) => encode_json(&OkResponse { ok: true }),
            Err(error) => {
                record_error("moltis_auth_rename_passkey", "AUTH_PASSKEY_RENAME_FAILED");
                encode_error("AUTH_PASSKEY_RENAME_FAILED", &error.to_string())
            },
        }
    })
}

/// Returns sandbox runtime status used by Settings > Sandboxes.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_status() -> *mut c_char {
    record_call("moltis_sandbox_status");
    trace_call("moltis_sandbox_status");

    with_ffi_boundary(|| {
        let config = moltis_config::discover_and_load();
        encode_json(&sandbox_status_from_config(&config))
    })
}

/// Returns cached tool and sandbox images.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_list_images() -> *mut c_char {
    record_call("moltis_sandbox_list_images");
    trace_call("moltis_sandbox_list_images");

    with_ffi_boundary(|| {
        let builder = moltis_tools::image_cache::DockerImageBuilder::new();
        let (cached, sandbox) = BRIDGE.runtime.block_on(async {
            tokio::join!(
                builder.list_cached(),
                moltis_tools::sandbox::list_sandbox_images()
            )
        });

        let mut images = Vec::new();

        if let Ok(list) = cached {
            images.extend(list.into_iter().map(|img| SandboxImageEntry {
                tag: img.tag,
                size: img.size,
                created: img.created,
                kind: "tool".to_owned(),
            }));
        }

        if let Ok(list) = sandbox {
            images.extend(list.into_iter().map(|img| SandboxImageEntry {
                tag: img.tag,
                size: img.size,
                created: img.created,
                kind: "sandbox".to_owned(),
            }));
        }

        encode_json(&SandboxImagesResponse { images })
    })
}

/// Deletes one cached image by tag.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_delete_image(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_sandbox_delete_image");
    trace_call("moltis_sandbox_delete_image");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SandboxDeleteImageRequest>(
            "moltis_sandbox_delete_image",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let tag = request.tag.trim();
        if tag.is_empty() {
            record_error("moltis_sandbox_delete_image", "IMAGE_TAG_REQUIRED");
            return encode_error("IMAGE_TAG_REQUIRED", "tag is required");
        }

        let result = BRIDGE.runtime.block_on(async {
            if tag.contains("-sandbox:") {
                moltis_tools::sandbox::remove_sandbox_image(tag).await
            } else {
                let builder = moltis_tools::image_cache::DockerImageBuilder::new();
                let full_tag = if tag.starts_with("moltis-cache/") {
                    tag.to_owned()
                } else {
                    format!("moltis-cache/{tag}")
                };
                builder.remove_cached(&full_tag).await
            }
        });

        match result {
            Ok(()) => encode_json(&OkResponse { ok: true }),
            Err(error) => {
                record_error("moltis_sandbox_delete_image", IMAGE_CACHE_DELETE_FAILED);
                encode_error(IMAGE_CACHE_DELETE_FAILED, &error.to_string())
            },
        }
    })
}

/// Removes all cached tool and sandbox images.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_prune_images() -> *mut c_char {
    record_call("moltis_sandbox_prune_images");
    trace_call("moltis_sandbox_prune_images");

    with_ffi_boundary(|| {
        let builder = moltis_tools::image_cache::DockerImageBuilder::new();
        let (tool_result, sandbox_result) = BRIDGE.runtime.block_on(async {
            tokio::join!(
                builder.prune_all(),
                moltis_tools::sandbox::clean_sandbox_images()
            )
        });

        let mut count = 0usize;
        if let Ok(n) = tool_result {
            count += n;
        }
        if let Ok(n) = sandbox_result {
            count += n;
        }

        if let (Err(e1), Err(e2)) = (&tool_result, &sandbox_result) {
            let message = format!("tool images: {e1}; sandbox images: {e2}");
            record_error("moltis_sandbox_prune_images", IMAGE_CACHE_PRUNE_FAILED);
            return encode_error(IMAGE_CACHE_PRUNE_FAILED, &message);
        }

        encode_json(&SandboxPruneImagesResponse { pruned: count })
    })
}

/// Checks package presence in a base Docker image.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_check_packages(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_sandbox_check_packages");
    trace_call("moltis_sandbox_check_packages");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SandboxCheckPackagesRequest>(
            "moltis_sandbox_check_packages",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let base = request
            .base
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("ubuntu:25.10")
            .to_owned();
        let packages: Vec<String> = request
            .packages
            .into_iter()
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty())
            .collect();

        if packages.is_empty() {
            return encode_json(&SandboxCheckPackagesResponse {
                found: HashMap::new(),
            });
        }

        if !is_valid_image_ref(&base) {
            record_error("moltis_sandbox_check_packages", SANDBOX_BASE_IMAGE_INVALID);
            return encode_error(
                SANDBOX_BASE_IMAGE_INVALID,
                "base image contains invalid characters",
            );
        }

        if let Some(bad) = packages.iter().find(|p| !is_valid_package_name(p)) {
            record_error(
                "moltis_sandbox_check_packages",
                SANDBOX_PACKAGE_NAME_INVALID,
            );
            return encode_error(
                SANDBOX_PACKAGE_NAME_INVALID,
                &format!("invalid package name: {bad}"),
            );
        }

        let checks: Vec<String> = packages
            .iter()
            .map(|pkg| {
                format!(
                    r#"if dpkg -s '{pkg}' >/dev/null 2>&1 || command -v '{pkg}' >/dev/null 2>&1; then echo "FOUND:{pkg}"; fi"#
                )
            })
            .collect();
        let script = checks.join("\n");

        let cli = moltis_tools::sandbox::container_cli();
        let output = BRIDGE.runtime.block_on(async {
            tokio::process::Command::new(cli)
                .args(["run", "--rm", "--entrypoint", "sh", &base, "-c", &script])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .await
        });

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let mut found = HashMap::new();
                for pkg in packages {
                    let present = stdout
                        .lines()
                        .any(|line| line.trim() == format!("FOUND:{pkg}"));
                    found.insert(pkg, present);
                }
                encode_json(&SandboxCheckPackagesResponse { found })
            },
            Err(error) => {
                record_error(
                    "moltis_sandbox_check_packages",
                    SANDBOX_CHECK_PACKAGES_FAILED,
                );
                encode_error(SANDBOX_CHECK_PACKAGES_FAILED, &error.to_string())
            },
        }
    })
}

/// Builds a sandbox image from base image + apt package list.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_build_image(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_sandbox_build_image");
    trace_call("moltis_sandbox_build_image");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SandboxBuildImageRequest>(
            "moltis_sandbox_build_image",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let name = request.name.trim();
        if name.is_empty() {
            record_error("moltis_sandbox_build_image", SANDBOX_IMAGE_NAME_REQUIRED);
            return encode_error(SANDBOX_IMAGE_NAME_REQUIRED, "name is required");
        }

        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            record_error("moltis_sandbox_build_image", SANDBOX_IMAGE_NAME_INVALID);
            return encode_error(
                SANDBOX_IMAGE_NAME_INVALID,
                "name must be alphanumeric, dash, or underscore",
            );
        }

        let base = request
            .base
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("ubuntu:25.10")
            .to_owned();
        let packages: Vec<String> = request
            .packages
            .into_iter()
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty())
            .collect();

        if !is_valid_image_ref(&base) {
            record_error("moltis_sandbox_build_image", SANDBOX_BASE_IMAGE_INVALID);
            return encode_error(
                SANDBOX_BASE_IMAGE_INVALID,
                "base image contains invalid characters",
            );
        }

        if packages.is_empty() {
            record_error(
                "moltis_sandbox_build_image",
                SANDBOX_IMAGE_PACKAGES_REQUIRED,
            );
            return encode_error(SANDBOX_IMAGE_PACKAGES_REQUIRED, "packages list is empty");
        }

        if let Some(bad) = packages.iter().find(|p| !is_valid_package_name(p)) {
            record_error("moltis_sandbox_build_image", SANDBOX_PACKAGE_NAME_INVALID);
            return encode_error(
                SANDBOX_PACKAGE_NAME_INVALID,
                &format!("invalid package name: {bad}"),
            );
        }

        let pkg_list = packages.join(" ");
        let dockerfile_contents = format!(
            "FROM {base}\n\
RUN apt-get update && apt-get install -y {pkg_list}\n\
RUN mkdir -p /home/sandbox\n\
ENV HOME=/home/sandbox\n\
WORKDIR /home/sandbox\n"
        );

        let tmp_dir = std::env::temp_dir().join(format!("moltis-build-{}", uuid::Uuid::new_v4()));
        if let Err(error) = std::fs::create_dir_all(&tmp_dir) {
            record_error("moltis_sandbox_build_image", SANDBOX_TMP_DIR_CREATE_FAILED);
            return encode_error(SANDBOX_TMP_DIR_CREATE_FAILED, &error.to_string());
        }

        let dockerfile_path = tmp_dir.join("Dockerfile");
        if let Err(error) = std::fs::write(&dockerfile_path, &dockerfile_contents) {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            record_error(
                "moltis_sandbox_build_image",
                SANDBOX_DOCKERFILE_WRITE_FAILED,
            );
            return encode_error(SANDBOX_DOCKERFILE_WRITE_FAILED, &error.to_string());
        }

        let builder = moltis_tools::image_cache::DockerImageBuilder::new();
        let result =
            BRIDGE
                .runtime
                .block_on(builder.ensure_image(name, &dockerfile_path, &tmp_dir));
        let _ = std::fs::remove_dir_all(&tmp_dir);

        match result {
            Ok(tag) => encode_json(&SandboxBuildImageResponse { tag }),
            Err(error) => {
                record_error("moltis_sandbox_build_image", SANDBOX_IMAGE_BUILD_FAILED);
                encode_error(SANDBOX_IMAGE_BUILD_FAILED, &error.to_string())
            },
        }
    })
}

/// Returns the effective default sandbox image.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_get_default_image() -> *mut c_char {
    record_call("moltis_sandbox_get_default_image");
    trace_call("moltis_sandbox_get_default_image");

    with_ffi_boundary(|| {
        let config = moltis_config::discover_and_load();
        let image = sandbox_effective_default_image(&config);
        encode_json(&SandboxDefaultImageResponse { image })
    })
}

/// Sets a runtime default sandbox image override.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_set_default_image(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_sandbox_set_default_image");
    trace_call("moltis_sandbox_set_default_image");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SandboxSetDefaultImageRequest>(
            "moltis_sandbox_set_default_image",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let config = moltis_config::discover_and_load();
        if sandbox_backend_name(&config) == "none" {
            record_error(
                "moltis_sandbox_set_default_image",
                SANDBOX_BACKEND_UNAVAILABLE,
            );
            return encode_error(SANDBOX_BACKEND_UNAVAILABLE, "no sandbox backend available");
        }

        let value = request
            .image
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);

        *BRIDGE
            .sandbox_default_image_override
            .write()
            .unwrap_or_else(|e| e.into_inner()) = value;

        let image = sandbox_effective_default_image(&config);
        encode_json(&SandboxDefaultImageResponse { image })
    })
}

/// Returns shared `/home/sandbox` persistence config.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_get_shared_home() -> *mut c_char {
    record_call("moltis_sandbox_get_shared_home");
    trace_call("moltis_sandbox_get_shared_home");

    with_ffi_boundary(|| {
        let config = moltis_config::discover_and_load();
        let response = sandbox_shared_home_config_from_config(&config);
        encode_json(&response)
    })
}

/// Updates shared `/home/sandbox` persistence config.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_set_shared_home(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_sandbox_set_shared_home");
    trace_call("moltis_sandbox_set_shared_home");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SandboxSharedHomeUpdateRequest>(
            "moltis_sandbox_set_shared_home",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let path = request
            .path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        let update_result = moltis_config::update_config(|cfg| {
            cfg.tools.exec.sandbox.shared_home_dir = path.clone();
            if request.enabled {
                cfg.tools.exec.sandbox.home_persistence =
                    moltis_config::schema::HomePersistenceConfig::Shared;
            } else if matches!(
                cfg.tools.exec.sandbox.home_persistence,
                moltis_config::schema::HomePersistenceConfig::Shared
            ) {
                cfg.tools.exec.sandbox.home_persistence =
                    moltis_config::schema::HomePersistenceConfig::Off;
            }
        });

        match update_result {
            Ok(saved_path) => {
                let config = moltis_config::discover_and_load();
                let response = SandboxSharedHomeSaveResponse {
                    ok: true,
                    restart_required: true,
                    config_path: saved_path.display().to_string(),
                    config: sandbox_shared_home_config_from_config(&config),
                };
                encode_json(&response)
            },
            Err(error) => {
                record_error(
                    "moltis_sandbox_set_shared_home",
                    SANDBOX_SHARED_HOME_SAVE_FAILED,
                );
                encode_error(SANDBOX_SHARED_HOME_SAVE_FAILED, &error.to_string())
            },
        }
    })
}

/// Returns running containers for the configured sandbox prefix.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_list_containers() -> *mut c_char {
    record_call("moltis_sandbox_list_containers");
    trace_call("moltis_sandbox_list_containers");

    with_ffi_boundary(|| {
        let config = moltis_config::discover_and_load();
        let prefix = sandbox_container_prefix(&config);
        match BRIDGE
            .runtime
            .block_on(moltis_tools::sandbox::list_running_containers(&prefix))
        {
            Ok(containers) => encode_json(&SandboxContainersResponse { containers }),
            Err(error) => {
                record_error(
                    "moltis_sandbox_list_containers",
                    SANDBOX_CONTAINERS_LIST_FAILED,
                );
                encode_error(SANDBOX_CONTAINERS_LIST_FAILED, &error.to_string())
            },
        }
    })
}

/// Stops one sandbox container.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_stop_container(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_sandbox_stop_container");
    trace_call("moltis_sandbox_stop_container");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SandboxContainerNameRequest>(
            "moltis_sandbox_stop_container",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let name = request.name.trim();
        if name.is_empty() {
            record_error(
                "moltis_sandbox_stop_container",
                "SANDBOX_CONTAINER_NAME_REQUIRED",
            );
            return encode_error("SANDBOX_CONTAINER_NAME_REQUIRED", "name is required");
        }

        let config = moltis_config::discover_and_load();
        let prefix = sandbox_container_prefix(&config);
        if !name.starts_with(&prefix) {
            record_error(
                "moltis_sandbox_stop_container",
                SANDBOX_CONTAINER_PREFIX_MISMATCH,
            );
            return encode_error(
                SANDBOX_CONTAINER_PREFIX_MISMATCH,
                "container name does not match expected prefix",
            );
        }

        match BRIDGE
            .runtime
            .block_on(moltis_tools::sandbox::stop_container(name))
        {
            Ok(()) => encode_json(&OkResponse { ok: true }),
            Err(error) => {
                record_error(
                    "moltis_sandbox_stop_container",
                    SANDBOX_CONTAINER_STOP_FAILED,
                );
                encode_error(SANDBOX_CONTAINER_STOP_FAILED, &error.to_string())
            },
        }
    })
}

/// Removes one sandbox container.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_remove_container(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_sandbox_remove_container");
    trace_call("moltis_sandbox_remove_container");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SandboxContainerNameRequest>(
            "moltis_sandbox_remove_container",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let name = request.name.trim();
        if name.is_empty() {
            record_error(
                "moltis_sandbox_remove_container",
                "SANDBOX_CONTAINER_NAME_REQUIRED",
            );
            return encode_error("SANDBOX_CONTAINER_NAME_REQUIRED", "name is required");
        }

        let config = moltis_config::discover_and_load();
        let prefix = sandbox_container_prefix(&config);
        if !name.starts_with(&prefix) {
            record_error(
                "moltis_sandbox_remove_container",
                SANDBOX_CONTAINER_PREFIX_MISMATCH,
            );
            return encode_error(
                SANDBOX_CONTAINER_PREFIX_MISMATCH,
                "container name does not match expected prefix",
            );
        }

        match BRIDGE
            .runtime
            .block_on(moltis_tools::sandbox::remove_container(name))
        {
            Ok(()) => encode_json(&OkResponse { ok: true }),
            Err(error) => {
                record_error(
                    "moltis_sandbox_remove_container",
                    SANDBOX_CONTAINER_REMOVE_FAILED,
                );
                encode_error(SANDBOX_CONTAINER_REMOVE_FAILED, &error.to_string())
            },
        }
    })
}

/// Stops and removes all sandbox containers.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_clean_containers() -> *mut c_char {
    record_call("moltis_sandbox_clean_containers");
    trace_call("moltis_sandbox_clean_containers");

    with_ffi_boundary(|| {
        let config = moltis_config::discover_and_load();
        let prefix = sandbox_container_prefix(&config);
        match BRIDGE
            .runtime
            .block_on(moltis_tools::sandbox::clean_all_containers(&prefix))
        {
            Ok(removed) => encode_json(&SandboxCleanContainersResponse { ok: true, removed }),
            Err(error) => {
                record_error(
                    "moltis_sandbox_clean_containers",
                    SANDBOX_CONTAINERS_CLEAN_FAILED,
                );
                encode_error(SANDBOX_CONTAINERS_CLEAN_FAILED, &error.to_string())
            },
        }
    })
}

/// Returns container runtime disk usage.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_disk_usage() -> *mut c_char {
    record_call("moltis_sandbox_disk_usage");
    trace_call("moltis_sandbox_disk_usage");

    with_ffi_boundary(|| {
        match BRIDGE
            .runtime
            .block_on(moltis_tools::sandbox::container_disk_usage())
        {
            Ok(usage) => encode_json(&SandboxDiskUsageResponse { usage }),
            Err(error) => {
                record_error("moltis_sandbox_disk_usage", SANDBOX_DISK_USAGE_FAILED);
                encode_error(SANDBOX_DISK_USAGE_FAILED, &error.to_string())
            },
        }
    })
}

/// Restarts the container daemon.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_restart_daemon() -> *mut c_char {
    record_call("moltis_sandbox_restart_daemon");
    trace_call("moltis_sandbox_restart_daemon");

    with_ffi_boundary(|| {
        match BRIDGE
            .runtime
            .block_on(moltis_tools::sandbox::restart_container_daemon())
        {
            Ok(()) => encode_json(&OkResponse { ok: true }),
            Err(error) => {
                record_error(
                    "moltis_sandbox_restart_daemon",
                    SANDBOX_DAEMON_RESTART_FAILED,
                );
                encode_error(SANDBOX_DAEMON_RESTART_FAILED, &error.to_string())
            },
        }
    })
}

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_shutdown() {
    record_call("moltis_shutdown");
    trace_call("moltis_shutdown");
    emit_log("INFO", "bridge", "Shutdown requested");

    // Stop the HTTP server if it is running.
    let mut guard = HTTPD.lock().unwrap_or_else(|e| e.into_inner());
    let handle = guard.take();
    drop(guard);
    if let Some(handle) = handle {
        let message = format!("Stopping httpd on {} during shutdown", handle.addr);
        stop_httpd_handle(handle, "bridge", &message);
    }

    emit_log("INFO", "bridge", "Shutdown complete");
}

#[allow(unsafe_code)]
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
        assert_eq!(version, moltis_config::VERSION);

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
    #[serial_test::serial]
    fn chat_stream_sends_error_for_no_provider() {
        use std::sync::{Arc, Mutex};

        // Force a no-provider environment so this test exercises the
        // synchronous error callback path deterministically.
        let original_registry = {
            let mut guard = BRIDGE.registry.write().unwrap_or_else(|e| e.into_inner());
            std::mem::replace(&mut *guard, ProviderRegistry::empty())
        };
        struct RestoreRegistry(Option<ProviderRegistry>);
        impl Drop for RestoreRegistry {
            fn drop(&mut self) {
                if let Some(registry) = self.0.take() {
                    let mut guard = BRIDGE.registry.write().unwrap_or_else(|e| e.into_inner());
                    *guard = registry;
                }
            }
        }
        let _restore_registry = RestoreRegistry(Some(original_registry));

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

        // With an empty registry, this must error synchronously.
        let request = r#"{"message":"test","model":"nonexistent-model-xyz"}"#;
        let c_request = CString::new(request).unwrap_or_else(|e| panic!("{e}"));

        // SAFETY: valid C string, valid callback, valid user_data.
        unsafe {
            moltis_chat_stream(c_request.as_ptr(), test_callback, user_data);
        }

        let events = unsafe { Arc::from_raw(user_data as *const Mutex<Vec<String>>) };
        let received = events.lock().unwrap_or_else(|e| e.into_inner());

        assert!(
            !received.is_empty(),
            "should receive at least one stream event"
        );
        let parsed: Value =
            serde_json::from_str(&received[0]).unwrap_or_else(|e| panic!("bad json: {e}"));
        assert_eq!(
            parsed.get("type").and_then(Value::as_str),
            Some("error"),
            "expected an error event when no provider is available"
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

    // ── Config / Identity / Soul tests ──────────────────────────────────

    #[test]
    fn get_config_returns_config_and_paths() {
        let payload = json_from_ptr(moltis_get_config());

        assert!(
            payload.get("config").is_some(),
            "get_config should return a 'config' field"
        );
        assert!(
            payload.get("config_dir").and_then(Value::as_str).is_some(),
            "get_config should return config_dir"
        );
        assert!(
            payload.get("data_dir").and_then(Value::as_str).is_some(),
            "get_config should return data_dir"
        );

        // The config should be an object with expected top-level keys.
        let config = payload.get("config").unwrap_or_else(|| panic!("no config"));
        assert!(
            config.get("server").is_some(),
            "config should have a 'server' section"
        );
    }

    #[test]
    fn save_config_returns_error_for_null() {
        let payload = json_from_ptr(moltis_save_config(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }

    #[test]
    fn save_config_returns_error_for_invalid_json() {
        let bad = CString::new("not valid json").unwrap_or_else(|e| panic!("{e}"));
        let payload = json_from_ptr(moltis_save_config(bad.as_ptr()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "invalid_json");
    }

    #[test]
    fn memory_status_returns_expected_fields() {
        let payload = json_from_ptr(moltis_memory_status());

        assert!(
            payload.get("available").and_then(Value::as_bool).is_some(),
            "memory_status should return available"
        );
        assert!(
            payload.get("total_files").and_then(Value::as_u64).is_some(),
            "memory_status should return total_files"
        );
        assert!(
            payload
                .get("total_chunks")
                .and_then(Value::as_u64)
                .is_some(),
            "memory_status should return total_chunks"
        );
        assert!(
            payload
                .get("db_size_display")
                .and_then(Value::as_str)
                .is_some(),
            "memory_status should return db_size_display"
        );
    }

    #[test]
    fn memory_config_get_returns_expected_fields() {
        let payload = json_from_ptr(moltis_memory_config_get());

        assert!(
            payload.get("backend").and_then(Value::as_str).is_some(),
            "memory_config_get should return backend"
        );
        assert!(
            payload.get("citations").and_then(Value::as_str).is_some(),
            "memory_config_get should return citations"
        );
        assert!(
            payload
                .get("disable_rag")
                .and_then(Value::as_bool)
                .is_some(),
            "memory_config_get should return disable_rag"
        );
        assert!(
            payload
                .get("llm_reranking")
                .and_then(Value::as_bool)
                .is_some(),
            "memory_config_get should return llm_reranking"
        );
    }

    #[test]
    fn memory_config_update_returns_error_for_null() {
        let payload = json_from_ptr(moltis_memory_config_update(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }

    #[test]
    fn memory_config_update_round_trip() {
        let request = serde_json::json!({
            "backend": "builtin",
            "citations": "auto",
            "llm_reranking": false,
            "session_export": false
        })
        .to_string();
        let c_request = CString::new(request).unwrap_or_else(|e| panic!("{e}"));
        let payload = json_from_ptr(moltis_memory_config_update(c_request.as_ptr()));

        assert_eq!(
            payload.get("backend").and_then(Value::as_str),
            Some("builtin"),
        );
        assert_eq!(
            payload.get("citations").and_then(Value::as_str),
            Some("auto")
        );
    }

    #[test]
    fn memory_qmd_status_returns_expected_fields() {
        let payload = json_from_ptr(moltis_memory_qmd_status());

        assert!(
            payload
                .get("feature_enabled")
                .and_then(Value::as_bool)
                .is_some(),
            "memory_qmd_status should return feature_enabled"
        );
        assert!(
            payload.get("available").and_then(Value::as_bool).is_some(),
            "memory_qmd_status should return available"
        );
    }

    #[test]
    fn get_soul_returns_soul_field() {
        let payload = json_from_ptr(moltis_get_soul());

        // soul field should exist (may be null or a string)
        assert!(
            payload.get("soul").is_some(),
            "get_soul should return a 'soul' field"
        );
    }

    #[test]
    fn save_soul_returns_error_for_null() {
        let payload = json_from_ptr(moltis_save_soul(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }

    #[test]
    fn save_identity_returns_error_for_null() {
        let payload = json_from_ptr(moltis_save_identity(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }

    #[test]
    fn save_user_profile_returns_error_for_null() {
        let payload = json_from_ptr(moltis_save_user_profile(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }

    #[test]
    fn list_env_vars_returns_env_vars_and_vault_status() {
        let payload = json_from_ptr(moltis_list_env_vars());

        assert!(
            payload.get("env_vars").and_then(Value::as_array).is_some(),
            "list_env_vars should return env_vars array"
        );
        assert!(
            payload
                .get("vault_status")
                .and_then(Value::as_str)
                .is_some(),
            "list_env_vars should return vault_status"
        );
    }

    #[test]
    fn set_env_var_returns_error_for_null() {
        let payload = json_from_ptr(moltis_set_env_var(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }

    #[test]
    fn set_env_var_rejects_invalid_key() {
        let request = r#"{"key":"BAD-KEY","value":"secret"}"#;
        let c_request = CString::new(request).unwrap_or_else(|e| panic!("{e}"));
        let payload = json_from_ptr(moltis_set_env_var(c_request.as_ptr()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "ENV_KEY_INVALID");
    }

    #[test]
    fn delete_env_var_returns_error_for_null() {
        let payload = json_from_ptr(moltis_delete_env_var(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }

    #[test]
    fn set_and_delete_env_var_round_trip() {
        let key = format!("MACOS_TEST_{}", uuid::Uuid::new_v4().simple());
        let set_request = serde_json::json!({
            "key": key,
            "value": "secret-value"
        })
        .to_string();
        let c_set_request = CString::new(set_request).unwrap_or_else(|e| panic!("{e}"));
        let set_payload = json_from_ptr(moltis_set_env_var(c_set_request.as_ptr()));
        assert_eq!(set_payload.get("ok").and_then(Value::as_bool), Some(true));

        let list_payload = json_from_ptr(moltis_list_env_vars());
        let env_vars = list_payload
            .get("env_vars")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("env_vars missing"));
        let item = env_vars
            .iter()
            .find(|entry| entry.get("key").and_then(Value::as_str) == Some(key.as_str()))
            .unwrap_or_else(|| panic!("saved env var should appear in list"));
        let id = item
            .get("id")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| panic!("env var id should be present"));

        let delete_request = serde_json::json!({ "id": id }).to_string();
        let c_delete_request = CString::new(delete_request).unwrap_or_else(|e| panic!("{e}"));
        let delete_payload = json_from_ptr(moltis_delete_env_var(c_delete_request.as_ptr()));
        assert_eq!(
            delete_payload.get("ok").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn auth_status_returns_expected_fields() {
        let payload = json_from_ptr(moltis_auth_status());

        assert!(
            payload
                .get("auth_disabled")
                .and_then(Value::as_bool)
                .is_some(),
            "auth_status should return auth_disabled"
        );
        assert!(
            payload
                .get("has_password")
                .and_then(Value::as_bool)
                .is_some(),
            "auth_status should return has_password"
        );
        assert!(
            payload
                .get("has_passkeys")
                .and_then(Value::as_bool)
                .is_some(),
            "auth_status should return has_passkeys"
        );
        assert!(
            payload
                .get("setup_complete")
                .and_then(Value::as_bool)
                .is_some(),
            "auth_status should return setup_complete"
        );
    }

    #[test]
    fn auth_list_passkeys_returns_array() {
        let payload = json_from_ptr(moltis_auth_list_passkeys());
        assert!(
            payload.get("passkeys").and_then(Value::as_array).is_some(),
            "auth_list_passkeys should return passkeys"
        );
    }

    #[test]
    fn auth_password_change_rejects_short_password() {
        let request = r#"{"new_password":"short"}"#;
        let c_request = CString::new(request).unwrap_or_else(|e| panic!("{e}"));
        let payload = json_from_ptr(moltis_auth_password_change(c_request.as_ptr()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "AUTH_PASSWORD_TOO_SHORT");
    }

    #[test]
    fn auth_remove_passkey_returns_error_for_null() {
        let payload = json_from_ptr(moltis_auth_remove_passkey(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }
}
