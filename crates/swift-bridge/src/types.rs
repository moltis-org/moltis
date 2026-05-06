//! Request, response, and bridge-level types for the Swift FFI layer.

use std::collections::HashMap;

use {
    moltis_sessions::metadata::SessionEntry,
    secrecy::Secret,
    serde::{Deserialize, Serialize},
};

// ── HTTP Server types ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct StartHttpdRequest {
    #[serde(default = "default_httpd_host")]
    pub host: String,
    #[serde(default = "default_httpd_port")]
    pub port: u16,
    #[serde(default)]
    pub config_dir: Option<String>,
    #[serde(default)]
    pub data_dir: Option<String>,
}

pub(crate) fn default_httpd_host() -> String {
    "127.0.0.1".to_owned()
}

pub(crate) fn default_httpd_port() -> u16 {
    8080
}

#[derive(Debug, Serialize)]
pub(crate) struct HttpdStatusResponse {
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addr: Option<String>,
}

// ── Chat request / response types ────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct ChatRequest {
    pub message: String,
    #[serde(default)]
    pub model: Option<String>,
    /// Reserved for future provider-hint resolution; deserialized so Swift
    /// can pass it but not yet used for routing.
    #[serde(default)]
    #[allow(dead_code)]
    pub provider: Option<String>,
    #[serde(default)]
    pub config_toml: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatResponse {
    pub reply: String,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub config_dir: String,
    pub default_soul: String,
    pub validation: Option<ValidationSummary>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ValidationSummary {
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
    pub has_errors: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct VersionResponse {
    pub bridge_version: &'static str,
    pub moltis_version: &'static str,
    pub config_dir: String,
}

// ── Session types ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct SwitchSessionRequest {
    pub key: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateSessionRequest {
    #[serde(default)]
    pub label: Option<String>,
}

/// Compact session entry for the Swift side.
#[derive(Debug, Serialize)]
pub(crate) struct BridgeSessionEntry {
    pub key: String,
    pub label: Option<String>,
    pub message_count: u32,
    pub created_at: u64,
    pub updated_at: u64,
    pub preview: Option<String>,
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
pub(crate) struct BridgeSessionHistory {
    pub entry: BridgeSessionEntry,
    pub messages: Vec<serde_json::Value>,
}

/// Chat request with session key.
#[derive(Debug, Deserialize)]
pub(crate) struct SessionChatRequest {
    pub session_key: String,
    pub message: String,
    #[serde(default)]
    pub model: Option<String>,
}

// ── Error envelope ───────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub(crate) struct ErrorEnvelope<'a> {
    pub error: ErrorPayload<'a>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ErrorPayload<'a> {
    pub code: &'a str,
    pub message: &'a str,
}

// ── Provider types ───────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub(crate) struct BridgeKnownProvider {
    pub name: &'static str,
    pub display_name: &'static str,
    pub auth_type: &'static str,
    pub env_key: Option<&'static str>,
    pub default_base_url: Option<&'static str>,
    pub requires_model: bool,
    pub key_optional: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct BridgeDetectedSource {
    pub provider: String,
    pub source: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct BridgeModelInfo {
    pub id: String,
    pub provider: String,
    pub display_name: String,
    pub created_at: Option<i64>,
    pub recommended: bool,
}

#[derive(Deserialize)]
pub(crate) struct SaveProviderRequest {
    pub provider: String,
    #[serde(default)]
    pub api_key: Option<Secret<String>>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub models: Option<Vec<String>>,
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
pub(crate) struct OkResponse {
    pub ok: bool,
}

// ── Config / Identity / Soul types ───────────────────────────────────────

#[derive(Debug, Serialize)]
pub(crate) struct GetConfigResponse {
    pub config: serde_json::Value,
    pub config_dir: String,
    pub data_dir: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct GetSoulResponse {
    pub soul: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SaveSoulRequest {
    #[serde(default)]
    pub soul: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SaveIdentityRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub theme: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SaveUserProfileRequest {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SetEnvVarRequest {
    pub key: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DeleteEnvVarRequest {
    pub id: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct ListEnvVarsResponse {
    pub env_vars: Vec<moltis_gateway::auth::EnvVarEntry>,
    pub vault_status: String,
}

// ── Memory types ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub(crate) struct MemoryStatusResponse {
    pub available: bool,
    pub total_files: usize,
    pub total_chunks: usize,
    pub db_size: u64,
    pub db_size_display: String,
    pub embedding_model: String,
    pub has_embeddings: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct MemoryConfigResponse {
    pub style: String,
    pub agent_write_mode: String,
    pub user_profile_write_mode: String,
    pub backend: String,
    pub provider: String,
    pub citations: String,
    pub disable_rag: bool,
    pub llm_reranking: bool,
    pub search_merge_strategy: String,
    pub session_export: String,
    pub prompt_memory_mode: String,
    pub qmd_feature_enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum SessionExportUpdateValue {
    Mode(String),
    LegacyBool(bool),
}

#[derive(Debug, Deserialize)]
pub(crate) struct MemoryConfigUpdateRequest {
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub agent_write_mode: Option<String>,
    #[serde(default)]
    pub user_profile_write_mode: Option<String>,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub citations: Option<String>,
    #[serde(default)]
    pub llm_reranking: Option<bool>,
    #[serde(default)]
    pub search_merge_strategy: Option<String>,
    #[serde(default)]
    pub disable_rag: Option<bool>,
    #[serde(default)]
    pub session_export: Option<SessionExportUpdateValue>,
    #[serde(default)]
    pub prompt_memory_mode: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct MemoryQmdStatusResponse {
    pub feature_enabled: bool,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ── Auth types ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub(crate) struct AuthStatusResponse {
    pub auth_disabled: bool,
    pub has_password: bool,
    pub has_passkeys: bool,
    pub setup_complete: bool,
}

#[derive(Deserialize)]
pub(crate) struct AuthPasswordChangeRequest {
    #[serde(default)]
    pub current_password: Option<Secret<String>>,
    pub new_password: Secret<String>,
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
pub(crate) struct AuthPasswordChangeResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_key: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AuthPasskeysResponse {
    pub passkeys: Vec<moltis_gateway::auth::PasskeyEntry>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AuthPasskeyIdRequest {
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AuthPasskeyRenameRequest {
    pub id: i64,
    pub name: String,
}

// ── Sandbox types ────────────────────────────────────────────────────────

pub(crate) const IMAGE_CACHE_DELETE_FAILED: &str = "IMAGE_CACHE_DELETE_FAILED";
pub(crate) const IMAGE_CACHE_PRUNE_FAILED: &str = "IMAGE_CACHE_PRUNE_FAILED";
pub(crate) const SANDBOX_CHECK_PACKAGES_FAILED: &str = "SANDBOX_CHECK_PACKAGES_FAILED";
pub(crate) const SANDBOX_BACKEND_UNAVAILABLE: &str = "SANDBOX_BACKEND_UNAVAILABLE";
pub(crate) const SANDBOX_IMAGE_NAME_REQUIRED: &str = "SANDBOX_IMAGE_NAME_REQUIRED";
pub(crate) const SANDBOX_IMAGE_PACKAGES_REQUIRED: &str = "SANDBOX_IMAGE_PACKAGES_REQUIRED";
pub(crate) const SANDBOX_IMAGE_NAME_INVALID: &str = "SANDBOX_IMAGE_NAME_INVALID";
pub(crate) const SANDBOX_TMP_DIR_CREATE_FAILED: &str = "SANDBOX_TMP_DIR_CREATE_FAILED";
pub(crate) const SANDBOX_DOCKERFILE_WRITE_FAILED: &str = "SANDBOX_DOCKERFILE_WRITE_FAILED";
pub(crate) const SANDBOX_IMAGE_BUILD_FAILED: &str = "SANDBOX_IMAGE_BUILD_FAILED";
pub(crate) const SANDBOX_CONTAINERS_LIST_FAILED: &str = "SANDBOX_CONTAINERS_LIST_FAILED";
pub(crate) const SANDBOX_CONTAINER_PREFIX_MISMATCH: &str = "SANDBOX_CONTAINER_PREFIX_MISMATCH";
pub(crate) const SANDBOX_CONTAINER_STOP_FAILED: &str = "SANDBOX_CONTAINER_STOP_FAILED";
pub(crate) const SANDBOX_CONTAINER_REMOVE_FAILED: &str = "SANDBOX_CONTAINER_REMOVE_FAILED";
pub(crate) const SANDBOX_CONTAINERS_CLEAN_FAILED: &str = "SANDBOX_CONTAINERS_CLEAN_FAILED";
pub(crate) const SANDBOX_DISK_USAGE_FAILED: &str = "SANDBOX_DISK_USAGE_FAILED";
pub(crate) const SANDBOX_DAEMON_RESTART_FAILED: &str = "SANDBOX_DAEMON_RESTART_FAILED";
pub(crate) const SANDBOX_SHARED_HOME_SAVE_FAILED: &str = "SANDBOX_SHARED_HOME_SAVE_FAILED";
pub(crate) const SANDBOX_PACKAGE_NAME_INVALID: &str = "SANDBOX_PACKAGE_NAME_INVALID";
pub(crate) const SANDBOX_BASE_IMAGE_INVALID: &str = "SANDBOX_BASE_IMAGE_INVALID";

/// Validates a package name to prevent shell injection.
/// Allows alphanumeric, hyphen, dot, plus, colon (covers dpkg naming conventions).
pub(crate) fn is_valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '+' | ':'))
}

/// Validates a container/base image reference (e.g. "ubuntu:25.10", "docker.io/library/ubuntu").
/// Allows alphanumeric, hyphen, dot, colon, slash, underscore.
pub(crate) fn is_valid_image_ref(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':' | '/' | '_'))
}

#[derive(Debug, Serialize)]
pub(crate) struct SandboxStatusResponse {
    pub backend: String,
    pub os: String,
    pub default_image: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SandboxImageEntry {
    pub tag: String,
    pub size: String,
    pub created: String,
    pub kind: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SandboxImagesResponse {
    pub images: Vec<SandboxImageEntry>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SandboxDeleteImageRequest {
    pub tag: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SandboxPruneImagesResponse {
    pub pruned: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SandboxCheckPackagesRequest {
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub packages: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SandboxCheckPackagesResponse {
    pub found: HashMap<String, bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SandboxBuildImageRequest {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub packages: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SandboxBuildImageResponse {
    pub tag: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SandboxDefaultImageResponse {
    pub image: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SandboxSetDefaultImageRequest {
    #[serde(default)]
    pub image: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SandboxSharedHomeConfigResponse {
    pub enabled: bool,
    pub mode: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SandboxSharedHomeUpdateRequest {
    pub enabled: bool,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SandboxSharedHomeSaveResponse {
    pub ok: bool,
    pub restart_required: bool,
    pub config_path: String,
    pub config: SandboxSharedHomeConfigResponse,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SandboxContainerNameRequest {
    pub name: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SandboxContainersResponse {
    pub containers: Vec<moltis_tools::sandbox::RunningContainer>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SandboxCleanContainersResponse {
    pub ok: bool,
    pub removed: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct SandboxDiskUsageResponse {
    pub usage: moltis_tools::sandbox::ContainerDiskUsage,
}
