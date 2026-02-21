//! GraphQL output and input types.
//!
//! These types are deserialized from the JSON values returned by service
//! methods. They use `#[derive(SimpleObject)]` for output types and
//! `#[derive(InputObject)]` for input types. Fields use `serde` for
//! deserialization and `async-graphql` for schema generation.
//!
//! For dynamic/untyped fields, the `Json` scalar is used.

use {
    async_graphql::{InputObject, SimpleObject},
    serde::Deserialize,
};

use crate::scalars::Json;

// ── Common result type ──────────────────────────────────────────────────────

/// Generic boolean result for mutations that return `{ "ok": true }`.
#[derive(Debug, SimpleObject, Deserialize)]
pub struct BoolResult {
    pub ok: bool,
}

// ── Health & Status ─────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthInfo {
    pub ok: bool,
    #[serde(default)]
    pub connections: Option<u64>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusInfo {
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub connections: Option<u64>,
    #[serde(default)]
    pub uptime_ms: Option<u64>,
}

// ── Sessions ────────────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEntry {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub created_at: Option<u64>,
    #[serde(default)]
    pub updated_at: Option<u64>,
    #[serde(default)]
    pub message_count: Option<u64>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub archived: Option<bool>,
    #[serde(default)]
    pub worktree_branch: Option<String>,
    #[serde(default)]
    pub sandbox_enabled: Option<bool>,
    #[serde(default)]
    pub sandbox_image: Option<String>,
    #[serde(default)]
    pub channel_binding: Option<String>,
    #[serde(default)]
    pub parent_session_key: Option<String>,
    #[serde(default)]
    pub fork_point: Option<u64>,
    #[serde(default)]
    pub mcp_disabled: Option<bool>,
    #[serde(default)]
    pub replying: Option<bool>,
}

#[derive(Debug, InputObject)]
pub struct SessionPatchInput {
    pub key: String,
    #[graphql(default)]
    pub label: Option<String>,
    #[graphql(default)]
    pub model: Option<String>,
    #[graphql(default)]
    pub archived: Option<bool>,
    #[graphql(default)]
    pub sandbox_enabled: Option<bool>,
    #[graphql(default)]
    pub sandbox_image: Option<String>,
    #[graphql(default)]
    pub mcp_disabled: Option<bool>,
}

#[derive(Debug, InputObject)]
pub struct SessionForkInput {
    pub key: String,
    #[graphql(default)]
    pub label: Option<String>,
    #[graphql(default)]
    pub at_index: Option<u64>,
}

#[derive(Debug, InputObject)]
pub struct SessionShareInput {
    pub key: String,
    #[graphql(default)]
    pub expires_hours: Option<u64>,
}

// ── Chat ────────────────────────────────────────────────────────────────────

#[derive(Debug, InputObject)]
pub struct ChatSendInput {
    pub message: String,
    #[graphql(default)]
    pub session_key: Option<String>,
    #[graphql(default)]
    pub model: Option<String>,
}

#[derive(Debug, InputObject)]
pub struct ChatInjectInput {
    pub role: String,
    pub content: String,
    #[graphql(default)]
    pub session_key: Option<String>,
}

// ── Cron ────────────────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJob {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub delete_after_run: Option<bool>,
    #[graphql(name = "schedule")]
    #[serde(default)]
    pub schedule: Option<Json>,
    #[graphql(name = "payload")]
    #[serde(default)]
    pub payload: Option<Json>,
    #[serde(default)]
    pub session_target: Option<String>,
    #[graphql(name = "state")]
    #[serde(default)]
    pub state: Option<Json>,
    #[serde(default)]
    pub created_at_ms: Option<u64>,
    #[serde(default)]
    pub updated_at_ms: Option<u64>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronStatus {
    #[serde(default)]
    pub running: Option<bool>,
    #[serde(default)]
    pub job_count: Option<u64>,
    #[serde(default)]
    pub enabled_count: Option<u64>,
    #[serde(default)]
    pub next_run_at_ms: Option<u64>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronRunRecord {
    #[serde(default)]
    pub job_id: Option<String>,
    #[serde(default)]
    pub started_at_ms: Option<u64>,
    #[serde(default)]
    pub finished_at_ms: Option<u64>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
}

// ── Projects ────────────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub directory: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub auto_worktree: Option<bool>,
    #[serde(default)]
    pub setup_command: Option<String>,
    #[serde(default)]
    pub teardown_command: Option<String>,
    #[serde(default)]
    pub branch_prefix: Option<String>,
    #[serde(default)]
    pub sandbox_image: Option<String>,
    #[serde(default)]
    pub detected: Option<bool>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, InputObject)]
pub struct ProjectInput {
    #[graphql(default)]
    pub id: Option<String>,
    pub label: String,
    pub directory: String,
    #[graphql(default)]
    pub system_prompt: Option<String>,
    #[graphql(default)]
    pub auto_worktree: Option<bool>,
    #[graphql(default)]
    pub setup_command: Option<String>,
    #[graphql(default)]
    pub teardown_command: Option<String>,
    #[graphql(default)]
    pub branch_prefix: Option<String>,
    #[graphql(default)]
    pub sandbox_image: Option<String>,
}

// ── Channels ────────────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInfo {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
}

// ── Providers & Models ──────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub configured: Option<bool>,
    #[serde(default)]
    pub auth_method: Option<String>,
    #[serde(default)]
    pub models: Option<Vec<String>>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub supports_tools: Option<bool>,
    #[serde(default)]
    pub supports_vision: Option<bool>,
    #[serde(default)]
    pub supports_streaming: Option<bool>,
    #[serde(default)]
    pub context_window: Option<u64>,
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
}

// ── Skills ──────────────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfo {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[graphql(name = "source")]
    #[serde(default)]
    pub source: Option<Json>,
    #[serde(default)]
    pub protected: Option<bool>,
    #[serde(default)]
    pub eligible: Option<bool>,
    #[serde(default)]
    pub missing_bins: Option<Vec<String>>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepo {
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub repo_name: Option<String>,
    #[serde(default)]
    pub installed_at_ms: Option<u64>,
    #[serde(default)]
    pub commit_sha: Option<String>,
    #[serde(default)]
    pub skill_count: Option<u64>,
    #[serde(default)]
    pub enabled_count: Option<u64>,
    #[serde(default)]
    pub format: Option<String>,
}

// ── MCP ─────────────────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub transport: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub tool_count: Option<u64>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTool {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub server: Option<String>,
}

// ── Voice / TTS / STT ───────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsStatus {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub provider: Option<String>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SttStatus {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub provider: Option<String>,
}

// ── Usage ───────────────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStatus {
    #[serde(default)]
    pub total_input_tokens: Option<u64>,
    #[serde(default)]
    pub total_output_tokens: Option<u64>,
    #[serde(default)]
    pub session_count: Option<u64>,
}

// ── Hooks ───────────────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookInfo {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub events: Option<Vec<String>>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub eligible: Option<bool>,
    #[serde(default)]
    pub call_count: Option<u64>,
    #[serde(default)]
    pub failure_count: Option<u64>,
}

// ── Agents ──────────────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentIdentity {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub creature: Option<String>,
    #[serde(default)]
    pub vibe: Option<String>,
}

#[derive(Debug, InputObject)]
pub struct AgentIdentityInput {
    #[graphql(default)]
    pub name: Option<String>,
    #[graphql(default)]
    pub emoji: Option<String>,
    #[graphql(default)]
    pub creature: Option<String>,
    #[graphql(default)]
    pub vibe: Option<String>,
}

// ── Memory ──────────────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStatus {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub file_count: Option<u64>,
    #[serde(default)]
    pub chunk_count: Option<u64>,
    #[serde(default)]
    pub backend: Option<String>,
}

// ── Subscription event types ────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GenericEvent {
    #[graphql(name = "data")]
    #[serde(flatten)]
    pub data: Json,
}

impl From<serde_json::Value> for GenericEvent {
    fn from(v: serde_json::Value) -> Self {
        Self { data: Json(v) }
    }
}

/// System heartbeat tick event with timestamp and memory stats.
#[derive(Debug, SimpleObject, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TickEvent {
    /// Unix timestamp in milliseconds.
    pub ts: u64,
    /// Memory usage statistics.
    pub mem: MemoryStats,
}

/// Memory usage breakdown.
#[derive(Debug, SimpleObject, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStats {
    /// Process RSS in bytes.
    pub process: u64,
    /// System available memory in bytes.
    pub available: u64,
    /// System total memory in bytes.
    pub total: u64,
}

// Allow `Json` to be used as a SimpleObject field (it implements OutputType via Scalar).
// serde `Deserialize` impl for Json so it can be deserialized from service responses.
impl<'de> Deserialize<'de> for Json {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = serde_json::Value::deserialize(deserializer)?;
        Ok(Json(v))
    }
}

impl Clone for Json {
    fn clone(&self) -> Self {
        Json(self.0.clone())
    }
}

impl std::fmt::Debug for Json {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Json({:?})", self.0)
    }
}
