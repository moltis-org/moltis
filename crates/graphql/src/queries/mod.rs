//! GraphQL query resolvers, organized by RPC namespace.

use async_graphql::{Context, Object, Result};

use crate::{
    rpc_call, rpc_json_call,
    scalars::Json,
    types::{
        AgentIdentity, BoolResult, ChannelInfo, ChannelSendersResult, ChatRawPrompt, CronJob,
        CronRunRecord, CronStatus, ExecApprovalConfig, ExecNodeConfig, HealthInfo, HeartbeatStatus,
        HookInfo, LocalSystemInfo, LogListResult, LogStatus, LogTailResult, McpServer, McpTool,
        MemoryConfig, MemoryStatus, ModelInfo, NodeDescription, NodeInfo, Project, ProjectContext,
        ProviderInfo, SecurityScanResult, SecurityStatus, SessionActiveResult, SessionBranch,
        SessionEntry, SessionShareResult, SkillInfo, SkillRepo, StatusInfo, SttStatus, SystemPresence,
        TtsStatus,
        UsageCost, UsageStatus, VoiceConfig, VoicewakeConfig, VoxtralRequirements,
    },
};

// ── Root ────────────────────────────────────────────────────────────────────

/// Root query type composing all namespace queries.
#[derive(Default)]
pub struct QueryRoot;

#[Object]
impl QueryRoot {
    /// Gateway health check.
    async fn health(&self, ctx: &Context<'_>) -> Result<HealthInfo> {
        rpc_call!("health", ctx)
    }

    /// Gateway status with hostname, version, connections, uptime.
    async fn status(&self, ctx: &Context<'_>) -> Result<StatusInfo> {
        rpc_call!("status", ctx)
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
    async fn presence(&self, ctx: &Context<'_>) -> Result<SystemPresence> {
        rpc_call!("system-presence", ctx)
    }

    /// Last activity duration for the current client.
    async fn last_heartbeat(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_call!("last-heartbeat", ctx)
    }
}

// ── Node ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct NodeQuery;

#[Object]
impl NodeQuery {
    /// List all connected nodes.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<NodeInfo>> {
        rpc_call!("node.list", ctx)
    }

    /// Get detailed info for a specific node.
    async fn describe(&self, ctx: &Context<'_>, node_id: String) -> Result<NodeDescription> {
        rpc_call!(
            "node.describe",
            ctx,
            serde_json::json!({ "nodeId": node_id })
        )
    }

    /// List pending pairing requests.
    async fn pair_requests(&self, ctx: &Context<'_>) -> Result<Json> {
        // Pairing request shape varies by transport.
        rpc_json_call!("node.pair.list", ctx)
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
        rpc_json_call!(
            "chat.history",
            ctx,
            serde_json::json!({ "sessionKey": session_key })
        )
    }

    /// Get chat context data.
    async fn context(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<Json> {
        // Dynamic context shape (system prompt, tools, etc.).
        rpc_json_call!(
            "chat.context",
            ctx,
            serde_json::json!({ "sessionKey": session_key })
        )
    }

    /// Get rendered system prompt.
    async fn raw_prompt(
        &self,
        ctx: &Context<'_>,
        session_key: Option<String>,
    ) -> Result<ChatRawPrompt> {
        rpc_call!(
            "chat.raw_prompt",
            ctx,
            serde_json::json!({ "sessionKey": session_key })
        )
    }

    /// Get full context with rendering (OpenAI messages format).
    async fn full_context(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<Json> {
        // OpenAI messages format — deeply nested, dynamic.
        rpc_json_call!(
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
        rpc_call!("sessions.list", ctx)
    }

    /// Preview a session without switching.
    async fn preview(&self, ctx: &Context<'_>, key: String) -> Result<SessionEntry> {
        rpc_call!("sessions.preview", ctx, serde_json::json!({ "key": key }))
    }

    /// Search sessions by query.
    async fn search(&self, ctx: &Context<'_>, query: String) -> Result<Vec<SessionEntry>> {
        rpc_call!(
            "sessions.search",
            ctx,
            serde_json::json!({ "query": query })
        )
    }

    /// Resolve or auto-create a session.
    async fn resolve(&self, ctx: &Context<'_>, key: String) -> Result<SessionEntry> {
        rpc_call!("sessions.resolve", ctx, serde_json::json!({ "key": key }))
    }

    /// Get session branches.
    async fn branches(&self, ctx: &Context<'_>, key: Option<String>) -> Result<Vec<SessionBranch>> {
        rpc_call!("sessions.branches", ctx, serde_json::json!({ "key": key }))
    }

    /// List shared session links.
    async fn shares(
        &self,
        ctx: &Context<'_>,
        key: Option<String>,
    ) -> Result<Vec<SessionShareResult>> {
        rpc_call!(
            "sessions.share.list",
            ctx,
            serde_json::json!({ "key": key })
        )
    }

    /// Whether this session has an active run (LLM is responding).
    async fn active(
        &self,
        ctx: &Context<'_>,
        session_key: String,
    ) -> Result<SessionActiveResult> {
        rpc_call!(
            "sessions.active",
            ctx,
            serde_json::json!({ "sessionKey": session_key })
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
        rpc_call!("channels.status", ctx)
    }

    /// List all channels.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<ChannelInfo>> {
        rpc_call!("channels.list", ctx)
    }

    /// List pending channel senders.
    async fn senders(&self, ctx: &Context<'_>) -> Result<ChannelSendersResult> {
        rpc_call!("channels.senders.list", ctx, serde_json::json!({}))
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
        rpc_json_call!("config.get", ctx, serde_json::json!({ "path": path }))
    }

    /// Get config schema definition. Returns dynamic JSON schema.
    async fn schema(&self, ctx: &Context<'_>) -> Result<Json> {
        // JSON schema definition is inherently dynamic.
        rpc_json_call!("config.schema", ctx)
    }
}

// ── Cron ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct CronQuery;

#[Object]
impl CronQuery {
    /// List all cron jobs.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<CronJob>> {
        rpc_call!("cron.list", ctx)
    }

    /// Get cron status.
    async fn status(&self, ctx: &Context<'_>) -> Result<CronStatus> {
        rpc_call!("cron.status", ctx)
    }

    /// Get run history for a cron job.
    async fn runs(&self, ctx: &Context<'_>, job_id: String) -> Result<Vec<CronRunRecord>> {
        rpc_call!("cron.runs", ctx, serde_json::json!({ "jobId": job_id }))
    }
}

// ── Heartbeat ───────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct HeartbeatQuery;

#[Object]
impl HeartbeatQuery {
    /// Get heartbeat configuration and status.
    async fn status(&self, ctx: &Context<'_>) -> Result<HeartbeatStatus> {
        rpc_call!("heartbeat.status", ctx)
    }

    /// Get heartbeat run history.
    async fn runs(&self, ctx: &Context<'_>, limit: Option<u64>) -> Result<Vec<CronRunRecord>> {
        rpc_call!("heartbeat.runs", ctx, serde_json::json!({ "limit": limit }))
    }
}

// ── Logs ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct LogsQuery;

#[Object]
impl LogsQuery {
    /// Stream log tail.
    async fn tail(&self, ctx: &Context<'_>, lines: Option<u64>) -> Result<LogTailResult> {
        rpc_call!("logs.tail", ctx, serde_json::json!({ "limit": lines }))
    }

    /// List logs.
    async fn list(&self, ctx: &Context<'_>) -> Result<LogListResult> {
        rpc_call!("logs.list", ctx)
    }

    /// Get log status.
    async fn status(&self, ctx: &Context<'_>) -> Result<LogStatus> {
        rpc_call!("logs.status", ctx)
    }
}

// ── TTS ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct TtsQuery;

#[Object]
impl TtsQuery {
    /// Get TTS status.
    async fn status(&self, ctx: &Context<'_>) -> Result<TtsStatus> {
        rpc_call!("tts.status", ctx)
    }

    /// Get available TTS providers.
    async fn providers(&self, ctx: &Context<'_>) -> Result<Vec<ProviderInfo>> {
        rpc_call!("tts.providers", ctx)
    }

    /// Generate a TTS test phrase.
    async fn generate_phrase(&self, ctx: &Context<'_>) -> Result<String> {
        rpc_call!("tts.generate_phrase", ctx)
    }
}

// ── STT ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct SttQuery;

#[Object]
impl SttQuery {
    /// Get STT status.
    async fn status(&self, ctx: &Context<'_>) -> Result<SttStatus> {
        rpc_call!("stt.status", ctx)
    }

    /// Get available STT providers.
    async fn providers(&self, ctx: &Context<'_>) -> Result<Vec<ProviderInfo>> {
        rpc_call!("stt.providers", ctx)
    }
}

// ── Voice ───────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct VoiceQuery;

#[Object]
impl VoiceQuery {
    /// Get voice configuration.
    async fn config(&self, ctx: &Context<'_>) -> Result<VoiceConfig> {
        rpc_call!("voice.config.get", ctx)
    }

    /// Get all voice providers with availability detection.
    async fn providers(&self, ctx: &Context<'_>) -> Result<Vec<ProviderInfo>> {
        rpc_call!("voice.providers.all", ctx)
    }

    /// Fetch ElevenLabs voice catalog.
    async fn elevenlabs_catalog(&self, ctx: &Context<'_>) -> Result<Json> {
        // ElevenLabs voice catalog is a complex external API structure.
        rpc_json_call!("voice.elevenlabs.catalog", ctx)
    }

    /// Check Voxtral local setup requirements.
    async fn voxtral_requirements(&self, ctx: &Context<'_>) -> Result<VoxtralRequirements> {
        rpc_call!("voice.config.voxtral_requirements", ctx)
    }
}

// ── Skills ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct SkillsQuery;

#[Object]
impl SkillsQuery {
    /// List installed skills.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<SkillInfo>> {
        rpc_call!("skills.list", ctx)
    }

    /// Get skills system status.
    async fn status(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_call!("skills.status", ctx)
    }

    /// Get skills binaries.
    async fn bins(&self, ctx: &Context<'_>) -> Result<Json> {
        // Binary dependency info varies by platform.
        rpc_json_call!("skills.bins", ctx)
    }

    /// List skill repositories.
    async fn repos(&self, ctx: &Context<'_>) -> Result<Vec<SkillRepo>> {
        rpc_call!("skills.repos.list", ctx)
    }

    /// Get skill details.
    async fn detail(&self, ctx: &Context<'_>, name: String) -> Result<SkillInfo> {
        rpc_call!(
            "skills.skill.detail",
            ctx,
            serde_json::json!({ "name": name })
        )
    }

    /// Get security status.
    async fn security_status(&self, ctx: &Context<'_>) -> Result<SecurityStatus> {
        rpc_call!("skills.security.status", ctx)
    }

    /// Run security scan.
    async fn security_scan(&self, ctx: &Context<'_>) -> Result<SecurityScanResult> {
        rpc_call!("skills.security.scan", ctx)
    }
}

// ── Models ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ModelQuery;

#[Object]
impl ModelQuery {
    /// List enabled models.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<ModelInfo>> {
        rpc_call!("models.list", ctx)
    }

    /// List all available models.
    async fn list_all(&self, ctx: &Context<'_>) -> Result<Vec<ModelInfo>> {
        rpc_call!("models.list_all", ctx)
    }
}

// ── Providers ───────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ProviderQuery;

#[Object]
impl ProviderQuery {
    /// List available provider integrations.
    async fn available(&self, ctx: &Context<'_>) -> Result<Vec<ProviderInfo>> {
        rpc_call!("providers.available", ctx)
    }

    /// Get OAuth status.
    async fn oauth_status(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_call!("providers.oauth.status", ctx)
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
    async fn system_info(&self, ctx: &Context<'_>) -> Result<LocalSystemInfo> {
        rpc_call!("providers.local.system_info", ctx)
    }

    /// List available local models.
    async fn models(&self, ctx: &Context<'_>) -> Result<Vec<ModelInfo>> {
        rpc_call!("providers.local.models", ctx)
    }

    /// Get local LLM status.
    async fn status(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_call!("providers.local.status", ctx)
    }

    /// Search HuggingFace models.
    async fn search_hf(&self, ctx: &Context<'_>, query: String) -> Result<Json> {
        // HuggingFace search results have external API shape.
        rpc_json_call!(
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
        rpc_call!("mcp.list", ctx)
    }

    /// Get MCP system status.
    async fn status(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_call!("mcp.status", ctx, serde_json::json!({}))
    }

    /// Get MCP server tools.
    async fn tools(&self, ctx: &Context<'_>, name: Option<String>) -> Result<Vec<McpTool>> {
        rpc_call!("mcp.tools", ctx, serde_json::json!({ "name": name }))
    }
}

// ── Usage ───────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct UsageQuery;

#[Object]
impl UsageQuery {
    /// Get usage statistics.
    async fn status(&self, ctx: &Context<'_>) -> Result<UsageStatus> {
        rpc_call!("usage.status", ctx)
    }

    /// Calculate cost for a usage period.
    async fn cost(&self, ctx: &Context<'_>) -> Result<UsageCost> {
        rpc_call!("usage.cost", ctx, serde_json::json!({}))
    }
}

// ── Exec Approvals ──────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ExecApprovalQuery;

#[Object]
impl ExecApprovalQuery {
    /// Get execution approval settings.
    async fn get(&self, ctx: &Context<'_>) -> Result<ExecApprovalConfig> {
        rpc_call!("exec.approvals.get", ctx)
    }

    /// Get node-specific approval settings.
    async fn node_config(&self, ctx: &Context<'_>) -> Result<ExecNodeConfig> {
        rpc_call!("exec.approvals.node.get", ctx)
    }
}

// ── Projects ────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ProjectQuery;

#[Object]
impl ProjectQuery {
    /// List all projects.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<Project>> {
        rpc_call!("projects.list", ctx)
    }

    /// Get a project by ID.
    async fn get(&self, ctx: &Context<'_>, id: String) -> Result<Project> {
        rpc_call!("projects.get", ctx, serde_json::json!({ "id": id }))
    }

    /// Get project context.
    async fn context(&self, ctx: &Context<'_>, id: String) -> Result<ProjectContext> {
        rpc_call!("projects.context", ctx, serde_json::json!({ "id": id }))
    }

    /// Path completion for projects.
    async fn complete_path(&self, ctx: &Context<'_>, prefix: String) -> Result<Vec<String>> {
        rpc_call!(
            "projects.complete_path",
            ctx,
            serde_json::json!({ "partial": prefix })
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
        rpc_call!("memory.status", ctx)
    }

    /// Get memory configuration.
    async fn config(&self, ctx: &Context<'_>) -> Result<MemoryConfig> {
        rpc_call!("memory.config.get", ctx)
    }

    /// Get QMD status.
    async fn qmd_status(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_call!("memory.qmd.status", ctx)
    }
}

// ── Hooks ───────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct HooksQuery;

#[Object]
impl HooksQuery {
    /// List discovered hooks with stats.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<HookInfo>> {
        rpc_call!("hooks.list", ctx)
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
        rpc_json_call!("agents.list", ctx)
    }

    /// Get agent identity.
    async fn identity(&self, ctx: &Context<'_>) -> Result<AgentIdentity> {
        rpc_call!("agent.identity.get", ctx)
    }
}

// ── Voicewake ───────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct VoicewakeQuery;

#[Object]
impl VoicewakeQuery {
    /// Get wake word configuration.
    async fn get(&self, ctx: &Context<'_>) -> Result<VoicewakeConfig> {
        rpc_call!("voicewake.get", ctx)
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
        rpc_json_call!("device.pair.list", ctx)
    }
}
