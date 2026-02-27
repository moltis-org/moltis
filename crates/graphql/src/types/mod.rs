//! GraphQL output and input types.
//!
//! These types are deserialized from the JSON values returned by service
//! methods. They use `#[derive(SimpleObject)]` for output types and
//! `#[derive(InputObject)]` for input types. Fields use `serde` for
//! deserialization and `async-graphql` for schema generation.
//!
//! For dynamic/untyped fields, the `Json` scalar is used.

use {async_graphql::SimpleObject, serde::Deserialize};

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

// ── System Presence ─────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemPresence {
    #[serde(default)]
    pub clients: Vec<ClientInfo>,
    #[serde(default)]
    pub nodes: Vec<NodeInfo>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    #[serde(default)]
    pub conn_id: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub connected_at: Option<u64>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub conn_id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

// ── Sessions ────────────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEntry {
    #[serde(default)]
    pub id: Option<String>,
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
    pub last_seen_message_count: Option<u64>,
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
    pub preview: Option<String>,
    #[serde(default)]
    pub mcp_disabled: Option<bool>,
    #[serde(default)]
    pub replying: Option<bool>,
}

/// Whether a session currently has an active LLM run (waiting for response).
#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionActiveResult {
    pub active: bool,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionBranch {
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub fork_point: Option<u64>,
    #[serde(default)]
    pub message_count: Option<u64>,
    #[serde(default)]
    pub created_at: Option<u64>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionShareResult {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub session_key: Option<String>,
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub views: Option<u64>,
    #[serde(default)]
    pub created_at: Option<u64>,
    #[serde(default)]
    pub revoked_at: Option<u64>,
    #[serde(default)]
    pub snapshot_message_count: Option<u64>,
    #[serde(default)]
    pub access_key: Option<String>,
    #[serde(default)]
    pub notice: Option<String>,
}

// ── Chat ────────────────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRawPrompt {
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub char_count: Option<u64>,
    #[serde(default, alias = "native_tools")]
    pub native_tools: Option<bool>,
    #[serde(default)]
    pub tool_count: Option<u64>,
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

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatStatus {
    #[serde(default)]
    pub config: Option<HeartbeatConfig>,
    #[serde(default)]
    pub job: Option<CronJob>,
    #[serde(default)]
    pub prompt_source: Option<String>,
    #[serde(default)]
    pub heartbeat_file_exists: Option<bool>,
    #[serde(default)]
    pub has_prompt: Option<bool>,
}

#[derive(Debug, SimpleObject, Deserialize)]
pub struct HeartbeatConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub every: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub ack_max_chars: Option<u64>,
    #[serde(default)]
    pub active_hours: Option<HeartbeatActiveHours>,
    #[serde(default)]
    pub sandbox_enabled: Option<bool>,
    #[serde(default)]
    pub sandbox_image: Option<String>,
}

#[derive(Debug, SimpleObject, Deserialize)]
pub struct HeartbeatActiveHours {
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub end: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
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

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectContext {
    #[serde(default)]
    pub project: Option<Project>,
    #[serde(default, alias = "context_files")]
    pub context_files: Option<Vec<ContextFile>>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextFile {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
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

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelSender {
    #[serde(default, alias = "peerId")]
    pub peer_id: Option<String>,
    #[serde(default, alias = "senderName")]
    pub sender_name: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default, alias = "messageCount")]
    pub message_count: Option<u64>,
    #[serde(default, alias = "lastSeen")]
    pub last_seen: Option<u64>,
    #[serde(default)]
    pub allowed: Option<bool>,
    #[serde(default, alias = "otpPending")]
    pub otp_pending: Option<ChannelOtpPending>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelOtpPending {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default, alias = "expiresAt")]
    pub expires_at: Option<u64>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelSendersResult {
    #[serde(default)]
    pub senders: Vec<ChannelSender>,
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
    #[serde(default, alias = "displayName")]
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

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSystemInfo {
    #[serde(default)]
    pub total_ram_gb: Option<f64>,
    #[serde(default)]
    pub available_ram_gb: Option<f64>,
    #[serde(default)]
    pub has_metal: Option<bool>,
    #[serde(default)]
    pub has_cuda: Option<bool>,
    #[serde(default)]
    pub has_gpu: Option<bool>,
    #[serde(default)]
    pub is_apple_silicon: Option<bool>,
    #[serde(default)]
    pub memory_tier: Option<String>,
    #[serde(default)]
    pub recommended_backend: Option<String>,
    #[serde(default)]
    pub available_backends: Option<Vec<LocalBackendInfo>>,
    #[serde(default)]
    pub backend_note: Option<String>,
    #[serde(default)]
    pub mlx_available: Option<bool>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendInfo {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub available: Option<bool>,
    #[serde(default)]
    pub install_commands: Option<Vec<String>>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderOAuthStartResult {
    #[serde(default)]
    pub auth_url: Option<String>,
    #[serde(default)]
    pub device_flow: Option<bool>,
    #[serde(default)]
    pub already_authenticated: Option<bool>,
    #[serde(default)]
    pub user_code: Option<String>,
    #[serde(default)]
    pub verification_uri: Option<String>,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthStartResult {
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default)]
    pub oauth_pending: Option<bool>,
    #[serde(default)]
    pub auth_url: Option<String>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelTestResult {
    pub ok: bool,
    #[serde(default)]
    pub model_id: Option<String>,
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

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionResult {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub duration_seconds: Option<f64>,
    #[serde(default)]
    pub words: Option<Vec<TranscriptionWord>>,
}

#[derive(Debug, SimpleObject, Deserialize)]
pub struct TranscriptionWord {
    #[serde(default)]
    pub word: Option<String>,
    #[serde(default)]
    pub start: Option<f64>,
    #[serde(default)]
    pub end: Option<f64>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsConvertResult {
    #[serde(default)]
    pub audio: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub size: Option<u64>,
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

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCost {
    #[serde(default)]
    pub cost: Option<f64>,
}

// ── Logs ────────────────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    #[serde(default, alias = "timestamp")]
    pub ts: Option<u64>,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub fields: Option<Json>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogTailResult {
    #[serde(default)]
    pub entries: Vec<LogEntry>,
    #[serde(default)]
    pub subscribed: Option<bool>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogListResult {
    #[serde(default)]
    pub entries: Vec<LogEntry>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogStatus {
    #[serde(default, alias = "unseen_warns")]
    pub unseen_warns: Option<u64>,
    #[serde(default, alias = "unseen_errors")]
    pub unseen_errors: Option<u64>,
    #[serde(default, alias = "enabled_levels")]
    pub enabled_levels: Option<LogEnabledLevels>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEnabledLevels {
    #[serde(default)]
    pub debug: Option<bool>,
    #[serde(default)]
    pub trace: Option<bool>,
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

// ── Exec Approvals ──────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecApprovalConfig {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub security_level: Option<String>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecNodeConfig {
    #[serde(default)]
    pub mode: Option<String>,
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
    pub theme: Option<String>,
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

// ── Voicewake ───────────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoicewakeConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
}

// ── Node Description ────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeDescription {
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub capabilities: Option<Vec<String>>,
    #[serde(default)]
    pub commands: Option<Vec<String>>,
    #[graphql(name = "permissions")]
    #[serde(default)]
    pub permissions: Option<Json>,
    #[serde(default)]
    pub path_env: Option<String>,
    #[serde(default)]
    pub remote_ip: Option<String>,
    #[serde(default)]
    pub connected_at: Option<u64>,
}

// ── Voice Config ────────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceConfig {
    #[serde(default)]
    pub tts: Option<VoiceTtsConfig>,
    #[serde(default)]
    pub stt: Option<VoiceSttConfig>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceTtsConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub elevenlabs_configured: Option<bool>,
    #[serde(default)]
    pub openai_configured: Option<bool>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSttConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub whisper_configured: Option<bool>,
    #[serde(default)]
    pub groq_configured: Option<bool>,
    #[serde(default)]
    pub deepgram_configured: Option<bool>,
    #[serde(default)]
    pub google_configured: Option<bool>,
    #[serde(default)]
    pub elevenlabs_configured: Option<bool>,
    #[serde(default)]
    pub whisper_cli_configured: Option<bool>,
    #[serde(default)]
    pub sherpa_onnx_configured: Option<bool>,
}

// ── Voxtral Requirements ────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoxtralRequirements {
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
    #[serde(default)]
    pub python: Option<VoxtralPythonStatus>,
    #[serde(default)]
    pub cuda: Option<VoxtralCudaStatus>,
    #[serde(default)]
    pub compatible: Option<bool>,
    #[serde(default)]
    pub reasons: Option<Vec<String>>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoxtralPythonStatus {
    #[serde(default)]
    pub available: Option<bool>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub sufficient: Option<bool>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoxtralCudaStatus {
    #[serde(default)]
    pub available: Option<bool>,
    #[serde(default)]
    pub gpu_name: Option<String>,
    #[serde(default)]
    pub memory_mb: Option<u64>,
    #[serde(default)]
    pub sufficient: Option<bool>,
}

// ── Skills Security ─────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityStatus {
    #[serde(default)]
    pub mcp_scan_available: Option<bool>,
    #[serde(default)]
    pub uvx_available: Option<bool>,
    #[serde(default)]
    pub supported: Option<bool>,
    #[serde(default)]
    pub installed_skills_dir: Option<String>,
    #[serde(default)]
    pub install_hint: Option<String>,
}

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityScanResult {
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default)]
    pub message: Option<String>,
    /// Raw mcp-scan output (external tool, variable shape).
    #[graphql(name = "results")]
    #[serde(default)]
    pub results: Option<Json>,
    #[serde(default)]
    pub installed_skills_dir: Option<String>,
}

// ── Memory Config ───────────────────────────────────────────────────────────

#[derive(Debug, SimpleObject, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryConfig {
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub citations: Option<String>,
    #[serde(default)]
    pub disable_rag: Option<bool>,
    #[serde(default)]
    pub llm_reranking: Option<bool>,
    #[serde(default)]
    pub session_export: Option<bool>,
    #[serde(default)]
    pub qmd_feature_enabled: Option<bool>,
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
