//! GraphQL query resolvers, organized by service namespace.
//!
//! Resolvers call domain services directly through the `Services` bundle —
//! no RPC string-based dispatch or `ServiceCaller` indirection.

use async_graphql::{Context, Object, Result};

use crate::{
    error::{from_service, from_service_json},
    scalars::Json,
    services,
    types::{
        AgentIdentity, BoolResult, ChannelInfo, ChannelSendersResult, ChatRawPrompt, CronJob,
        CronRunRecord, CronStatus, ExecApprovalConfig, ExecNodeConfig, HealthInfo, HeartbeatStatus,
        HookInfo, LocalSystemInfo, LogListResult, LogStatus, LogTailResult, McpServer, McpTool,
        MemoryConfig, MemoryStatus, ModelInfo, NodeDescription, NodeInfo, Project, ProjectContext,
        ProviderInfo, SecurityScanResult, SecurityStatus, SessionActiveResult, SessionBranch,
        SessionEntry, SessionShareResult, SkillInfo, SkillRepo, StatusInfo, SttStatus,
        SystemPresence, TtsStatus, UsageCost, UsageStatus, VoiceConfig, VoicewakeConfig,
        VoxtralRequirements,
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
        let s = services!(ctx);
        from_service(s.system_info.health().await)
    }

    /// Gateway status with hostname, version, connections, uptime.
    async fn status(&self, ctx: &Context<'_>) -> Result<StatusInfo> {
        let s = services!(ctx);
        from_service(s.system_info.status().await)
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
        let s = services!(ctx);
        from_service(s.system_info.system_presence().await)
    }

    /// Last activity duration for the current client.
    async fn last_heartbeat(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.system_info.health().await)
    }
}

// ── Node ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct NodeQuery;

#[Object]
impl NodeQuery {
    /// List all connected nodes.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<NodeInfo>> {
        let s = services!(ctx);
        from_service(s.system_info.node_list().await)
    }

    /// Get detailed info for a specific node.
    async fn describe(&self, ctx: &Context<'_>, node_id: String) -> Result<NodeDescription> {
        let s = services!(ctx);
        from_service(
            s.system_info
                .node_describe(serde_json::json!({ "nodeId": node_id }))
                .await,
        )
    }

    /// List pending pairing requests.
    async fn pair_requests(&self, ctx: &Context<'_>) -> Result<Json> {
        // Pairing request shape varies by transport.
        let s = services!(ctx);
        from_service_json(
            s.system_info
                .node_list()
                .await
                .map(|_| serde_json::json!([])),
        )
    }
}

// ── Chat ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ChatQuery;

#[Object]
impl ChatQuery {
    /// Get chat history for a session.
    async fn history(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<Json> {
        let s = services!(ctx);
        // Messages contain deeply nested tool calls, images, etc.
        from_service_json(
            s.chat
                .history(serde_json::json!({ "sessionKey": session_key }))
                .await,
        )
    }

    /// Get chat context data.
    async fn context(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<Json> {
        let s = services!(ctx);
        // Dynamic context shape (system prompt, tools, etc.).
        from_service_json(
            s.chat
                .context(serde_json::json!({ "sessionKey": session_key }))
                .await,
        )
    }

    /// Get rendered system prompt.
    async fn raw_prompt(
        &self,
        ctx: &Context<'_>,
        session_key: Option<String>,
    ) -> Result<ChatRawPrompt> {
        let s = services!(ctx);
        from_service(
            s.chat
                .raw_prompt(serde_json::json!({ "sessionKey": session_key }))
                .await,
        )
    }

    /// Get full context with rendering (OpenAI messages format).
    async fn full_context(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<Json> {
        let s = services!(ctx);
        // OpenAI messages format — deeply nested, dynamic.
        from_service_json(
            s.chat
                .full_context(serde_json::json!({ "sessionKey": session_key }))
                .await,
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
        let s = services!(ctx);
        from_service(s.session.list().await)
    }

    /// Preview a session without switching.
    async fn preview(&self, ctx: &Context<'_>, key: String) -> Result<SessionEntry> {
        let s = services!(ctx);
        from_service(s.session.preview(serde_json::json!({ "key": key })).await)
    }

    /// Search sessions by query.
    async fn search(&self, ctx: &Context<'_>, query: String) -> Result<Vec<SessionEntry>> {
        let s = services!(ctx);
        from_service(
            s.session
                .search(serde_json::json!({ "query": query }))
                .await,
        )
    }

    /// Resolve or auto-create a session.
    async fn resolve(&self, ctx: &Context<'_>, key: String) -> Result<SessionEntry> {
        let s = services!(ctx);
        from_service(s.session.resolve(serde_json::json!({ "key": key })).await)
    }

    /// Get session branches.
    async fn branches(&self, ctx: &Context<'_>, key: Option<String>) -> Result<Vec<SessionBranch>> {
        let s = services!(ctx);
        from_service(s.session.branches(serde_json::json!({ "key": key })).await)
    }

    /// List shared session links.
    async fn shares(
        &self,
        ctx: &Context<'_>,
        key: Option<String>,
    ) -> Result<Vec<SessionShareResult>> {
        let s = services!(ctx);
        from_service(
            s.session
                .share_list(serde_json::json!({ "key": key }))
                .await,
        )
    }

    /// Whether this session has an active run (LLM is responding).
    async fn active(&self, ctx: &Context<'_>, session_key: String) -> Result<SessionActiveResult> {
        let s = services!(ctx);
        from_service(
            s.chat
                .active(serde_json::json!({ "sessionKey": session_key }))
                .await,
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
        let s = services!(ctx);
        from_service(s.channel.status().await)
    }

    /// List all channels.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<ChannelInfo>> {
        let s = services!(ctx);
        from_service(s.channel.status().await)
    }

    /// List pending channel senders.
    async fn senders(&self, ctx: &Context<'_>) -> Result<ChannelSendersResult> {
        let s = services!(ctx);
        from_service(s.channel.senders_list(serde_json::json!({})).await)
    }
}

// ── Config ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ConfigQuery;

#[Object]
impl ConfigQuery {
    /// Get config value at a path. Returns dynamic user-defined config data.
    async fn get(&self, ctx: &Context<'_>, path: Option<String>) -> Result<Json> {
        let s = services!(ctx);
        // User config values are arbitrary types.
        from_service_json(s.config.get(serde_json::json!({ "path": path })).await)
    }

    /// Get config schema definition. Returns dynamic JSON schema.
    async fn schema(&self, ctx: &Context<'_>) -> Result<Json> {
        let s = services!(ctx);
        // JSON schema definition is inherently dynamic.
        from_service_json(s.config.schema().await)
    }
}

// ── Cron ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct CronQuery;

#[Object]
impl CronQuery {
    /// List all cron jobs.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<CronJob>> {
        let s = services!(ctx);
        from_service(s.cron.list().await)
    }

    /// Get cron status.
    async fn status(&self, ctx: &Context<'_>) -> Result<CronStatus> {
        let s = services!(ctx);
        from_service(s.cron.status().await)
    }

    /// Get run history for a cron job.
    async fn runs(&self, ctx: &Context<'_>, job_id: String) -> Result<Vec<CronRunRecord>> {
        let s = services!(ctx);
        from_service(s.cron.runs(serde_json::json!({ "jobId": job_id })).await)
    }
}

// ── Heartbeat ───────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct HeartbeatQuery;

#[Object]
impl HeartbeatQuery {
    /// Get heartbeat configuration and status.
    async fn status(&self, ctx: &Context<'_>) -> Result<HeartbeatStatus> {
        let s = services!(ctx);
        from_service(s.system_info.heartbeat_status().await)
    }

    /// Get heartbeat run history.
    async fn runs(&self, ctx: &Context<'_>, limit: Option<u64>) -> Result<Vec<CronRunRecord>> {
        let s = services!(ctx);
        from_service(
            s.system_info
                .heartbeat_runs(serde_json::json!({ "limit": limit }))
                .await,
        )
    }
}

// ── Logs ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct LogsQuery;

#[Object]
impl LogsQuery {
    /// Stream log tail.
    async fn tail(&self, ctx: &Context<'_>, lines: Option<u64>) -> Result<LogTailResult> {
        let s = services!(ctx);
        from_service(s.logs.tail(serde_json::json!({ "limit": lines })).await)
    }

    /// List logs.
    async fn list(&self, ctx: &Context<'_>) -> Result<LogListResult> {
        let s = services!(ctx);
        from_service(s.logs.list(serde_json::json!({})).await)
    }

    /// Get log status.
    async fn status(&self, ctx: &Context<'_>) -> Result<LogStatus> {
        let s = services!(ctx);
        from_service(s.logs.status().await)
    }
}

// ── TTS ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct TtsQuery;

#[Object]
impl TtsQuery {
    /// Get TTS status.
    async fn status(&self, ctx: &Context<'_>) -> Result<TtsStatus> {
        let s = services!(ctx);
        from_service(s.tts.status().await)
    }

    /// Get available TTS providers.
    async fn providers(&self, ctx: &Context<'_>) -> Result<Vec<ProviderInfo>> {
        let s = services!(ctx);
        from_service(s.tts.providers().await)
    }

    /// Generate a TTS test phrase.
    async fn generate_phrase(&self, _ctx: &Context<'_>) -> Result<String> {
        Ok("Hello, how can I help you today?".to_string())
    }
}

// ── STT ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct SttQuery;

#[Object]
impl SttQuery {
    /// Get STT status.
    async fn status(&self, ctx: &Context<'_>) -> Result<SttStatus> {
        let s = services!(ctx);
        from_service(s.stt.status().await)
    }

    /// Get available STT providers.
    async fn providers(&self, ctx: &Context<'_>) -> Result<Vec<ProviderInfo>> {
        let s = services!(ctx);
        from_service(s.stt.providers().await)
    }
}

// ── Voice ───────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct VoiceQuery;

#[Object]
impl VoiceQuery {
    /// Get voice configuration.
    async fn config(&self, _ctx: &Context<'_>) -> Result<VoiceConfig> {
        // Voice config is managed at the gateway level; return default.
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    /// Get all voice providers with availability detection.
    async fn providers(&self, _ctx: &Context<'_>) -> Result<Vec<ProviderInfo>> {
        // Voice providers are gateway-level; return empty.
        from_service(Ok(serde_json::json!([])))
    }

    /// Fetch ElevenLabs voice catalog.
    async fn elevenlabs_catalog(&self, _ctx: &Context<'_>) -> Result<Json> {
        // ElevenLabs voice catalog is a complex external API structure.
        from_service_json(Ok(serde_json::json!([])))
    }

    /// Check Voxtral local setup requirements.
    async fn voxtral_requirements(&self, _ctx: &Context<'_>) -> Result<VoxtralRequirements> {
        from_service(Ok(serde_json::json!({ "ok": true })))
    }
}

// ── Skills ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct SkillsQuery;

#[Object]
impl SkillsQuery {
    /// List installed skills.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<SkillInfo>> {
        let s = services!(ctx);
        from_service(s.skills.list().await)
    }

    /// Get skills system status.
    async fn status(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.skills.status().await)
    }

    /// Get skills binaries.
    async fn bins(&self, ctx: &Context<'_>) -> Result<Json> {
        let s = services!(ctx);
        // Binary dependency info varies by platform.
        from_service_json(s.skills.bins().await)
    }

    /// List skill repositories.
    async fn repos(&self, ctx: &Context<'_>) -> Result<Vec<SkillRepo>> {
        let s = services!(ctx);
        from_service(s.skills.repos_list().await)
    }

    /// Get skill details.
    async fn detail(&self, ctx: &Context<'_>, name: String) -> Result<SkillInfo> {
        let s = services!(ctx);
        from_service(
            s.skills
                .skill_detail(serde_json::json!({ "name": name }))
                .await,
        )
    }

    /// Get security status.
    async fn security_status(&self, ctx: &Context<'_>) -> Result<SecurityStatus> {
        let s = services!(ctx);
        from_service(s.skills.security_status().await)
    }

    /// Run security scan.
    async fn security_scan(&self, ctx: &Context<'_>) -> Result<SecurityScanResult> {
        let s = services!(ctx);
        from_service(s.skills.security_scan().await)
    }
}

// ── Models ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ModelQuery;

#[Object]
impl ModelQuery {
    /// List enabled models.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<ModelInfo>> {
        let s = services!(ctx);
        from_service(s.model.list().await)
    }

    /// List all available models.
    async fn list_all(&self, ctx: &Context<'_>) -> Result<Vec<ModelInfo>> {
        let s = services!(ctx);
        from_service(s.model.list_all().await)
    }
}

// ── Providers ───────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ProviderQuery;

#[Object]
impl ProviderQuery {
    /// List available provider integrations.
    async fn available(&self, ctx: &Context<'_>) -> Result<Vec<ProviderInfo>> {
        let s = services!(ctx);
        from_service(s.provider_setup.available().await)
    }

    /// Get OAuth status.
    async fn oauth_status(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.provider_setup.oauth_status(serde_json::json!({})).await)
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
        let s = services!(ctx);
        from_service(s.local_llm.system_info().await)
    }

    /// List available local models.
    async fn models(&self, ctx: &Context<'_>) -> Result<Vec<ModelInfo>> {
        let s = services!(ctx);
        from_service(s.local_llm.models().await)
    }

    /// Get local LLM status.
    async fn status(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.local_llm.status().await)
    }

    /// Search HuggingFace models.
    async fn search_hf(&self, ctx: &Context<'_>, query: String) -> Result<Json> {
        let s = services!(ctx);
        // HuggingFace search results have external API shape.
        from_service_json(
            s.local_llm
                .search_hf(serde_json::json!({ "query": query }))
                .await,
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
        let s = services!(ctx);
        from_service(s.mcp.list().await)
    }

    /// Get MCP system status.
    async fn status(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.mcp.status(serde_json::json!({})).await)
    }

    /// Get MCP server tools.
    async fn tools(&self, ctx: &Context<'_>, name: Option<String>) -> Result<Vec<McpTool>> {
        let s = services!(ctx);
        from_service(s.mcp.tools(serde_json::json!({ "name": name })).await)
    }
}

// ── Usage ───────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct UsageQuery;

#[Object]
impl UsageQuery {
    /// Get usage statistics.
    async fn status(&self, ctx: &Context<'_>) -> Result<UsageStatus> {
        let s = services!(ctx);
        from_service(s.usage.status().await)
    }

    /// Calculate cost for a usage period.
    async fn cost(&self, ctx: &Context<'_>) -> Result<UsageCost> {
        let s = services!(ctx);
        from_service(s.usage.cost(serde_json::json!({})).await)
    }
}

// ── Exec Approvals ──────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ExecApprovalQuery;

#[Object]
impl ExecApprovalQuery {
    /// Get execution approval settings.
    async fn get(&self, ctx: &Context<'_>) -> Result<ExecApprovalConfig> {
        let s = services!(ctx);
        from_service(s.exec_approval.get().await)
    }

    /// Get node-specific approval settings.
    async fn node_config(&self, ctx: &Context<'_>) -> Result<ExecNodeConfig> {
        let s = services!(ctx);
        from_service(s.exec_approval.node_get(serde_json::json!({})).await)
    }
}

// ── Projects ────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ProjectQuery;

#[Object]
impl ProjectQuery {
    /// List all projects.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<Project>> {
        let s = services!(ctx);
        from_service(s.project.list().await)
    }

    /// Get a project by ID.
    async fn get(&self, ctx: &Context<'_>, id: String) -> Result<Project> {
        let s = services!(ctx);
        from_service(s.project.get(serde_json::json!({ "id": id })).await)
    }

    /// Get project context.
    async fn context(&self, ctx: &Context<'_>, id: String) -> Result<ProjectContext> {
        let s = services!(ctx);
        from_service(s.project.context(serde_json::json!({ "id": id })).await)
    }

    /// Path completion for projects.
    async fn complete_path(&self, ctx: &Context<'_>, prefix: String) -> Result<Vec<String>> {
        let s = services!(ctx);
        from_service(
            s.project
                .complete_path(serde_json::json!({ "partial": prefix }))
                .await,
        )
    }
}

// ── Memory ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct MemoryQuery;

#[Object]
impl MemoryQuery {
    /// Get memory system status.
    async fn status(&self, _ctx: &Context<'_>) -> Result<MemoryStatus> {
        from_service(Ok(serde_json::json!({ "enabled": false })))
    }

    /// Get memory configuration.
    async fn config(&self, _ctx: &Context<'_>) -> Result<MemoryConfig> {
        from_service(Ok(serde_json::json!({})))
    }

    /// Get QMD status.
    async fn qmd_status(&self, _ctx: &Context<'_>) -> Result<BoolResult> {
        from_service(Ok(serde_json::json!({ "available": false })))
    }
}

// ── Hooks ───────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct HooksQuery;

#[Object]
impl HooksQuery {
    /// List discovered hooks with stats.
    async fn list(&self, ctx: &Context<'_>) -> Result<Vec<HookInfo>> {
        let s = services!(ctx);
        from_service(s.system_info.hooks_list().await)
    }
}

// ── Agents ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct AgentQuery;

#[Object]
impl AgentQuery {
    /// List available agents.
    async fn list(&self, ctx: &Context<'_>) -> Result<Json> {
        let s = services!(ctx);
        // Agent list includes dynamic config/capabilities per agent.
        from_service_json(s.agent.list().await)
    }

    /// Get agent identity.
    async fn identity(&self, ctx: &Context<'_>) -> Result<AgentIdentity> {
        let s = services!(ctx);
        from_service(s.agent.identity_get().await)
    }
}

// ── Voicewake ───────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct VoicewakeQuery;

#[Object]
impl VoicewakeQuery {
    /// Get wake word configuration.
    async fn get(&self, ctx: &Context<'_>) -> Result<VoicewakeConfig> {
        let s = services!(ctx);
        from_service(s.voicewake.get().await)
    }
}

// ── Device ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct DeviceQuery;

#[Object]
impl DeviceQuery {
    /// List paired devices.
    async fn pair_requests(&self, _ctx: &Context<'_>) -> Result<Json> {
        // Device pairing info varies by transport type.
        from_service_json(Ok(serde_json::json!([])))
    }
}
