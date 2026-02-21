//! GraphQL query resolvers, organized by RPC namespace.

use std::sync::Arc;

use async_graphql::{Context, Object, Result};

use crate::{
    context::GqlContext,
    error::{gql_err, parse_err},
    scalars::Json,
    types::{
        AgentIdentity, BoolResult, ChannelInfo, CronJob, CronRunRecord, CronStatus, HealthInfo,
        HookInfo, McpServer, McpTool, MemoryStatus, ModelInfo, Project, ProviderInfo, SessionEntry,
        SkillInfo, SkillRepo, StatusInfo, SttStatus, TtsStatus, UsageStatus,
    },
};

// ── Helper macros ───────────────────────────────────────────────────────────

macro_rules! rpc_typed {
    ($method:expr, $ctx:expr) => {{
        let c = $ctx.data::<Arc<GqlContext>>()?;
        let r = c.rpc($method, serde_json::json!({})).await.map_err(gql_err)?;
        serde_json::from_value(r).map_err(parse_err)
    }};
    ($method:expr, $ctx:expr, $params:expr) => {{
        let c = $ctx.data::<Arc<GqlContext>>()?;
        let r = c.rpc($method, $params).await.map_err(gql_err)?;
        serde_json::from_value(r).map_err(parse_err)
    }};
}

/// Only for truly dynamic/untyped data (user config values, chat context payloads).
macro_rules! rpc_json {
    ($method:expr, $ctx:expr) => {{
        let c = $ctx.data::<Arc<GqlContext>>()?;
        let r = c.rpc($method, serde_json::json!({})).await.map_err(gql_err)?;
        Ok(Json(r))
    }};
    ($method:expr, $ctx:expr, $params:expr) => {{
        let c = $ctx.data::<Arc<GqlContext>>()?;
        let r = c.rpc($method, $params).await.map_err(gql_err)?;
        Ok(Json(r))
    }};
}

// ── Root ────────────────────────────────────────────────────────────────────

/// Root query type composing all namespace queries.
#[derive(Default)]
pub struct QueryRoot;

#[Object]
impl QueryRoot {
    /// Gateway health check.
    async fn health(&self, ctx: &Context<'_>) -> Result<HealthInfo> {
        rpc_typed!("health", ctx)
    }

    /// Gateway status with hostname, version, connections, uptime.
    async fn status(&self, ctx: &Context<'_>) -> Result<StatusInfo> {
        rpc_typed!("status", ctx)
    }

    /// System queries (presence, heartbeat).
    async fn system(&self) -> SystemQuery {
        SystemQuery
    }

    /// Node management queries.
    async fn node(&self) -> NodeQuery {
        NodeQuery
    }

    /// Chat queries (history, context).
    async fn chat(&self) -> ChatQuery {
        ChatQuery
    }

    /// Session queries.
    async fn sessions(&self) -> SessionQuery {
        SessionQuery
    }

    /// Channel queries.
    async fn channels(&self) -> ChannelQuery {
        ChannelQuery
    }

    /// Configuration queries.
    async fn config(&self) -> ConfigQuery {
        ConfigQuery
    }

    /// Cron job queries.
    async fn cron(&self) -> CronQuery {
        CronQuery
    }

    /// Heartbeat queries.
    async fn heartbeat(&self) -> HeartbeatQuery {
        HeartbeatQuery
    }

    /// Log queries.
    async fn logs(&self) -> LogsQuery {
        LogsQuery
    }

    /// TTS queries.
    async fn tts(&self) -> TtsQuery {
        TtsQuery
    }

    /// STT queries.
    async fn stt(&self) -> SttQuery {
        SttQuery
    }

    /// Voice configuration queries.
    async fn voice(&self) -> VoiceQuery {
        VoiceQuery
    }

    /// Skills queries.
    async fn skills(&self) -> SkillsQuery {
        SkillsQuery
    }

    /// Model queries.
    async fn models(&self) -> ModelQuery {
        ModelQuery
    }

    /// Provider queries.
    async fn providers(&self) -> ProviderQuery {
        ProviderQuery
    }

    /// MCP server queries.
    async fn mcp(&self) -> McpQuery {
        McpQuery
    }

    /// Usage and cost queries.
    async fn usage(&self) -> UsageQuery {
        UsageQuery
    }

    /// Execution approval queries.
    async fn exec_approvals(&self) -> ExecApprovalQuery {
        ExecApprovalQuery
    }

    /// Project queries.
    async fn projects(&self) -> ProjectQuery {
        ProjectQuery
    }

    /// Memory system queries.
    async fn memory(&self) -> MemoryQuery {
        MemoryQuery
    }

    /// Hook queries.
    async fn hooks(&self) -> HooksQuery {
        HooksQuery
    }

    /// Agent queries.
    async fn agents(&self) -> AgentQuery {
        AgentQuery
    }

    /// Voicewake configuration.
    async fn voicewake(&self) -> VoicewakeQuery {
        VoicewakeQuery
    }

    /// Device pairing queries.
    async fn device(&self) -> DeviceQuery {
        DeviceQuery
    }
}

// ── System ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct SystemQuery;

#[Object]
impl SystemQuery {
    /// Detailed client and node presence information.
    async fn presence(&self, ctx: &Context<'_>) -> Result<Json> {
        // Presence payload varies by client/node structure — keep dynamic.
        rpc_json!("system-presence", ctx)
    }

    /// Last activity duration for the current client.
    async fn last_heartbeat(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_typed!("last-heartbeat", ctx)
    }
}

// ── Node ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct NodeQuery;

#[Object]
impl NodeQuery {
    /// List all connected nodes.
    async fn list(&self, ctx: &Context<'_>) -> Result<Json> {
        // NodeSession not Serialize; gateway manually builds JSON.
        rpc_json!("node.list", ctx)
    }

    /// Get detailed info for a specific node.
    async fn describe(&self, ctx: &Context<'_>, node_id: String) -> Result<Json> {
        // NodeSession not Serialize; gateway manually builds JSON.
        rpc_json!(
            "node.describe",
            ctx,
            serde_json::json!({ "nodeId": node_id })
        )
    }

    /// List pending pairing requests.
    async fn pair_requests(&self, ctx: &Context<'_>) -> Result<Json> {
        // Pairing request shape varies by transport.
        rpc_json!("node.pair.list", ctx)
    }
}

// ── Chat ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ChatQuery;

#[Object]
impl ChatQuery {
    /// Get chat history for a session.
    async fn history(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<Json> {
        // Messages contain deeply nested tool calls, images, etc.
        rpc_json!(
            "chat.history",
            ctx,
            serde_json::json!({ "sessionKey": session_key })
        )
    }

    /// Get chat context data.
    async fn context(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<Json> {
        // Dynamic context shape (system prompt, tools, etc.).
        rpc_json!(
            "chat.context",
            ctx,
            serde_json::json!({ "sessionKey": session_key })
        )
    }

    /// Get rendered system prompt.
    async fn raw_prompt(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<Json> {
        // Rendered prompt string or structured prompt sections.
        rpc_json!(
            "chat.raw_prompt",
            ctx,
            serde_json::json!({ "sessionKey": session_key })
        )
    }

    /// Get full context with rendering (OpenAI messages format).
    async fn full_context(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<Json> {
        // OpenAI messages format — deeply nested, dynamic.
        rpc_json!(
            "chat.full_context",
            ctx,
            serde_json::json!({ "sessionKey": session_key })
        )
    }
}

// ── Sessions ────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct SessionQuery;

#[Object]
impl SessionQuery {
    /// List all sessions.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<SessionEntry>> {
        rpc_typed!("sessions.list", ctx)
    }

    /// Preview a session without switching.
    async fn preview(&self, ctx: &Context<'_>, key: String) -> Result<SessionEntry> {
        rpc_typed!("sessions.preview", ctx, serde_json::json!({ "key": key }))
    }

    /// Search sessions by query.
    async fn search(&self, ctx: &Context<'_>, query: String) -> Result<Vec<SessionEntry>> {
        rpc_typed!(
            "sessions.search",
            ctx,
            serde_json::json!({ "query": query })
        )
    }

    /// Resolve or auto-create a session.
    async fn resolve(&self, ctx: &Context<'_>, key: String) -> Result<SessionEntry> {
        rpc_typed!("sessions.resolve", ctx, serde_json::json!({ "key": key }))
    }

    /// Get session branches.
    async fn branches(&self, ctx: &Context<'_>, key: Option<String>) -> Result<Json> {
        // Branch tree structure is recursive/variable.
        rpc_json!("sessions.branches", ctx, serde_json::json!({ "key": key }))
    }

    /// List shared session links.
    async fn shares(&self, ctx: &Context<'_>, key: Option<String>) -> Result<Json> {
        // Share link structure includes tokens and URLs.
        rpc_json!(
            "sessions.share.list",
            ctx,
            serde_json::json!({ "key": key })
        )
    }
}

// ── Channels ────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ChannelQuery;

#[Object]
impl ChannelQuery {
    /// Get channel status.
    async fn status(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_typed!("channels.status", ctx)
    }

    /// List all channels.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<ChannelInfo>> {
        rpc_typed!("channels.list", ctx)
    }

    /// List pending channel senders.
    async fn senders(&self, ctx: &Context<'_>) -> Result<Json> {
        // Sender approval info includes OTP/allowlist state.
        rpc_json!("channels.senders.list", ctx, serde_json::json!({}))
    }
}

// ── Config ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ConfigQuery;

#[Object]
impl ConfigQuery {
    /// Get config value at a path. Returns dynamic user-defined config data.
    async fn get(&self, ctx: &Context<'_>, path: Option<String>) -> Result<Json> {
        // User config values are arbitrary types.
        rpc_json!("config.get", ctx, serde_json::json!({ "path": path }))
    }

    /// Get config schema definition. Returns dynamic JSON schema.
    async fn schema(&self, ctx: &Context<'_>) -> Result<Json> {
        // JSON schema definition is inherently dynamic.
        rpc_json!("config.schema", ctx)
    }
}

// ── Cron ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct CronQuery;

#[Object]
impl CronQuery {
    /// List all cron jobs.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<CronJob>> {
        rpc_typed!("cron.list", ctx)
    }

    /// Get cron status.
    async fn status(&self, ctx: &Context<'_>) -> Result<CronStatus> {
        rpc_typed!("cron.status", ctx)
    }

    /// Get run history for a cron job.
    async fn runs(&self, ctx: &Context<'_>, job_id: String) -> Result<Vec<CronRunRecord>> {
        rpc_typed!("cron.runs", ctx, serde_json::json!({ "jobId": job_id }))
    }
}

// ── Heartbeat ───────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct HeartbeatQuery;

#[Object]
impl HeartbeatQuery {
    /// Get heartbeat configuration and status.
    async fn status(&self, ctx: &Context<'_>) -> Result<Json> {
        // Heartbeat config shape includes cron-like schedule fields.
        rpc_json!("heartbeat.status", ctx)
    }

    /// Get heartbeat run history.
    async fn runs(&self, ctx: &Context<'_>, limit: Option<u64>) -> Result<Vec<CronRunRecord>> {
        rpc_typed!("heartbeat.runs", ctx, serde_json::json!({ "limit": limit }))
    }
}

// ── Logs ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct LogsQuery;

#[Object]
impl LogsQuery {
    /// Stream log tail.
    async fn tail(&self, ctx: &Context<'_>, lines: Option<u64>) -> Result<Json> {
        // Log entries contain dynamic tracing fields.
        rpc_json!("logs.tail", ctx, serde_json::json!({ "lines": lines }))
    }

    /// List logs.
    async fn list(&self, ctx: &Context<'_>) -> Result<Json> {
        // Log entries contain dynamic tracing fields.
        rpc_json!("logs.list", ctx)
    }

    /// Get log status.
    async fn status(&self, ctx: &Context<'_>) -> Result<Json> {
        // Contains unseen_warns, unseen_errors, enabled_levels map.
        rpc_json!("logs.status", ctx)
    }
}

// ── TTS ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct TtsQuery;

#[Object]
impl TtsQuery {
    /// Get TTS status.
    async fn status(&self, ctx: &Context<'_>) -> Result<TtsStatus> {
        rpc_typed!("tts.status", ctx)
    }

    /// Get available TTS providers.
    async fn providers(&self, ctx: &Context<'_>) -> Result<Vec<ProviderInfo>> {
        rpc_typed!("tts.providers", ctx)
    }

    /// Generate a TTS test phrase.
    async fn generate_phrase(&self, ctx: &Context<'_>) -> Result<Json> {
        // Returns a plain string.
        rpc_json!("tts.generate_phrase", ctx)
    }
}

// ── STT ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct SttQuery;

#[Object]
impl SttQuery {
    /// Get STT status.
    async fn status(&self, ctx: &Context<'_>) -> Result<SttStatus> {
        rpc_typed!("stt.status", ctx)
    }

    /// Get available STT providers.
    async fn providers(&self, ctx: &Context<'_>) -> Result<Vec<ProviderInfo>> {
        rpc_typed!("stt.providers", ctx)
    }
}

// ── Voice ───────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct VoiceQuery;

#[Object]
impl VoiceQuery {
    /// Get voice configuration.
    async fn config(&self, ctx: &Context<'_>) -> Result<Json> {
        // Voice config is a complex nested structure with provider-specific fields.
        rpc_json!("voice.config.get", ctx)
    }

    /// Get all voice providers with availability detection.
    async fn providers(&self, ctx: &Context<'_>) -> Result<Vec<ProviderInfo>> {
        rpc_typed!("voice.providers.all", ctx)
    }

    /// Fetch ElevenLabs voice catalog.
    async fn elevenlabs_catalog(&self, ctx: &Context<'_>) -> Result<Json> {
        // ElevenLabs voice catalog is a complex external structure.
        rpc_json!("voice.elevenlabs.catalog", ctx)
    }

    /// Check Voxtral local setup requirements.
    async fn voxtral_requirements(&self, ctx: &Context<'_>) -> Result<Json> {
        // Contains platform-specific requirement checks.
        rpc_json!("voice.config.voxtral_requirements", ctx)
    }
}

// ── Skills ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct SkillsQuery;

#[Object]
impl SkillsQuery {
    /// List installed skills.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<SkillInfo>> {
        rpc_typed!("skills.list", ctx)
    }

    /// Get skills system status.
    async fn status(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_typed!("skills.status", ctx)
    }

    /// Get skills binaries.
    async fn bins(&self, ctx: &Context<'_>) -> Result<Json> {
        // Binary dependency info varies by platform.
        rpc_json!("skills.bins", ctx)
    }

    /// List skill repositories.
    async fn repos(&self, ctx: &Context<'_>) -> Result<Vec<SkillRepo>> {
        rpc_typed!("skills.repos.list", ctx)
    }

    /// Get skill details.
    async fn detail(&self, ctx: &Context<'_>, name: String) -> Result<SkillInfo> {
        rpc_typed!(
            "skills.skill.detail",
            ctx,
            serde_json::json!({ "name": name })
        )
    }

    /// Get security status.
    async fn security_status(&self, ctx: &Context<'_>) -> Result<Json> {
        // Security scan results with variable issue shapes.
        rpc_json!("skills.security.status", ctx)
    }

    /// Run security scan.
    async fn security_scan(&self, ctx: &Context<'_>) -> Result<Json> {
        // Security scan results with variable issue shapes.
        rpc_json!("skills.security.scan", ctx)
    }
}

// ── Models ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ModelQuery;

#[Object]
impl ModelQuery {
    /// List enabled models.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<ModelInfo>> {
        rpc_typed!("models.list", ctx)
    }

    /// List all available models.
    async fn list_all(&self, ctx: &Context<'_>) -> Result<Vec<ModelInfo>> {
        rpc_typed!("models.list_all", ctx)
    }
}

// ── Providers ───────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ProviderQuery;

#[Object]
impl ProviderQuery {
    /// List available provider integrations.
    async fn available(&self, ctx: &Context<'_>) -> Result<Vec<ProviderInfo>> {
        rpc_typed!("providers.available", ctx)
    }

    /// Get OAuth status.
    async fn oauth_status(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_typed!("providers.oauth.status", ctx)
    }

    /// Local LLM queries.
    async fn local(&self) -> LocalLlmQuery {
        LocalLlmQuery
    }
}

#[derive(Default)]
pub struct LocalLlmQuery;

#[Object]
impl LocalLlmQuery {
    /// Get system information for local LLM.
    async fn system_info(&self, ctx: &Context<'_>) -> Result<Json> {
        // System info includes platform-specific GPU/RAM details.
        rpc_json!("providers.local.system_info", ctx)
    }

    /// List available local models.
    async fn models(&self, ctx: &Context<'_>) -> Result<Vec<ModelInfo>> {
        rpc_typed!("providers.local.models", ctx)
    }

    /// Get local LLM status.
    async fn status(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_typed!("providers.local.status", ctx)
    }

    /// Search HuggingFace models.
    async fn search_hf(&self, ctx: &Context<'_>, query: String) -> Result<Json> {
        // HuggingFace search results have external API shape.
        rpc_json!(
            "providers.local.search_hf",
            ctx,
            serde_json::json!({ "query": query })
        )
    }
}

// ── MCP ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct McpQuery;

#[Object]
impl McpQuery {
    /// List MCP servers.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<McpServer>> {
        rpc_typed!("mcp.list", ctx)
    }

    /// Get MCP system status.
    async fn status(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_typed!("mcp.status", ctx, serde_json::json!({}))
    }

    /// Get MCP server tools.
    async fn tools(&self, ctx: &Context<'_>, name: Option<String>) -> Result<Vec<McpTool>> {
        rpc_typed!("mcp.tools", ctx, serde_json::json!({ "name": name }))
    }
}

// ── Usage ───────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct UsageQuery;

#[Object]
impl UsageQuery {
    /// Get usage statistics.
    async fn status(&self, ctx: &Context<'_>) -> Result<UsageStatus> {
        rpc_typed!("usage.status", ctx)
    }

    /// Calculate cost for a usage period.
    async fn cost(&self, ctx: &Context<'_>) -> Result<Json> {
        // Cost breakdown varies by billing model.
        rpc_json!("usage.cost", ctx, serde_json::json!({}))
    }
}

// ── Exec Approvals ──────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ExecApprovalQuery;

#[Object]
impl ExecApprovalQuery {
    /// Get execution approval settings.
    async fn get(&self, ctx: &Context<'_>) -> Result<Json> {
        // Approval config includes policy rules and patterns.
        rpc_json!("exec.approvals.get", ctx)
    }

    /// Get node-specific approval settings.
    async fn node_config(&self, ctx: &Context<'_>) -> Result<Json> {
        // Per-node approval config with variable rule shapes.
        rpc_json!("exec.approvals.node.get", ctx)
    }
}

// ── Projects ────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ProjectQuery;

#[Object]
impl ProjectQuery {
    /// List all projects.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<Project>> {
        rpc_typed!("projects.list", ctx)
    }

    /// Get a project by ID.
    async fn get(&self, ctx: &Context<'_>, id: String) -> Result<Project> {
        rpc_typed!("projects.get", ctx, serde_json::json!({ "id": id }))
    }

    /// Get project context.
    async fn context(&self, ctx: &Context<'_>, id: String) -> Result<Json> {
        // Project context includes dynamic file tree and environment.
        rpc_json!("projects.context", ctx, serde_json::json!({ "id": id }))
    }

    /// Path completion for projects.
    async fn complete_path(&self, ctx: &Context<'_>, prefix: String) -> Result<Json> {
        // Returns completion suggestions as variable-length array.
        rpc_json!(
            "projects.complete_path",
            ctx,
            serde_json::json!({ "prefix": prefix })
        )
    }
}

// ── Memory ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct MemoryQuery;

#[Object]
impl MemoryQuery {
    /// Get memory system status.
    async fn status(&self, ctx: &Context<'_>) -> Result<MemoryStatus> {
        rpc_typed!("memory.status", ctx)
    }

    /// Get memory configuration.
    async fn config(&self, ctx: &Context<'_>) -> Result<Json> {
        // Memory config includes backend-specific settings.
        rpc_json!("memory.config.get", ctx)
    }

    /// Get QMD status.
    async fn qmd_status(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_typed!("memory.qmd.status", ctx)
    }
}

// ── Hooks ───────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct HooksQuery;

#[Object]
impl HooksQuery {
    /// List discovered hooks with stats.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<HookInfo>> {
        rpc_typed!("hooks.list", ctx)
    }
}

// ── Agents ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct AgentQuery;

#[Object]
impl AgentQuery {
    /// List available agents.
    async fn list(&self, ctx: &Context<'_>) -> Result<Json> {
        // Agent list includes dynamic config/capabilities per agent.
        rpc_json!("agents.list", ctx)
    }

    /// Get agent identity.
    async fn identity(&self, ctx: &Context<'_>) -> Result<AgentIdentity> {
        rpc_typed!("agent.identity.get", ctx)
    }
}

// ── Voicewake ───────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct VoicewakeQuery;

#[Object]
impl VoicewakeQuery {
    /// Get wake word configuration.
    async fn get(&self, ctx: &Context<'_>) -> Result<Json> {
        // Wake word config includes platform-specific engine settings.
        rpc_json!("voicewake.get", ctx)
    }
}

// ── Device ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct DeviceQuery;

#[Object]
impl DeviceQuery {
    /// List paired devices.
    async fn pair_requests(&self, ctx: &Context<'_>) -> Result<Json> {
        // Device pairing info varies by transport type.
        rpc_json!("device.pair.list", ctx)
    }
}
