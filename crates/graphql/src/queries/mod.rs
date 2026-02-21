//! GraphQL query resolvers, organized by RPC namespace.

use std::sync::Arc;

use async_graphql::{Context, Object, Result};

use crate::{
    context::GqlContext,
    error::{gql_err, parse_err},
    scalars::Json,
    types::StatusInfo,
};

/// Root query type composing all namespace queries.
#[derive(Default)]
pub struct QueryRoot;

#[Object]
impl QueryRoot {
    /// Gateway health check.
    async fn health(&self, ctx: &Context<'_>) -> Result<Json> {
        let c = ctx.data::<Arc<GqlContext>>()?;
        let r = c
            .rpc("health", serde_json::json!({}))
            .await
            .map_err(gql_err)?;
        Ok(Json(r))
    }

    /// Gateway status with hostname, version, connections, uptime.
    async fn status(&self, ctx: &Context<'_>) -> Result<StatusInfo> {
        let c = ctx.data::<Arc<GqlContext>>()?;
        let r = c
            .rpc("status", serde_json::json!({}))
            .await
            .map_err(gql_err)?;
        serde_json::from_value(r).map_err(parse_err)
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

// ── Namespace query types ───────────────────────────────────────────────────

macro_rules! rpc_query {
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

#[derive(Default)]
pub struct SystemQuery;

#[Object]
impl SystemQuery {
    /// Detailed client and node presence information.
    async fn presence(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("system-presence", ctx)
    }

    /// Last activity duration for the current client.
    async fn last_heartbeat(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("last-heartbeat", ctx)
    }
}

#[derive(Default)]
pub struct NodeQuery;

#[Object]
impl NodeQuery {
    /// List all connected nodes.
    async fn list(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("node.list", ctx)
    }

    /// Get detailed info for a specific node.
    async fn describe(&self, ctx: &Context<'_>, node_id: String) -> Result<Json> {
        rpc_query!(
            "node.describe",
            ctx,
            serde_json::json!({ "nodeId": node_id })
        )
    }

    /// List pending pairing requests.
    async fn pair_requests(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("node.pair.list", ctx)
    }
}

#[derive(Default)]
pub struct ChatQuery;

#[Object]
impl ChatQuery {
    /// Get chat history for a session.
    async fn history(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<Json> {
        rpc_query!(
            "chat.history",
            ctx,
            serde_json::json!({ "sessionKey": session_key })
        )
    }

    /// Get chat context data.
    async fn context(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<Json> {
        rpc_query!(
            "chat.context",
            ctx,
            serde_json::json!({ "sessionKey": session_key })
        )
    }

    /// Get rendered system prompt.
    async fn raw_prompt(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<Json> {
        rpc_query!(
            "chat.raw_prompt",
            ctx,
            serde_json::json!({ "sessionKey": session_key })
        )
    }

    /// Get full context with rendering (OpenAI messages format).
    async fn full_context(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<Json> {
        rpc_query!(
            "chat.full_context",
            ctx,
            serde_json::json!({ "sessionKey": session_key })
        )
    }
}

#[derive(Default)]
pub struct SessionQuery;

#[Object]
impl SessionQuery {
    /// List all sessions.
    async fn list(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("sessions.list", ctx)
    }

    /// Preview a session without switching.
    async fn preview(&self, ctx: &Context<'_>, key: String) -> Result<Json> {
        rpc_query!("sessions.preview", ctx, serde_json::json!({ "key": key }))
    }

    /// Search sessions by query.
    async fn search(&self, ctx: &Context<'_>, query: String) -> Result<Json> {
        rpc_query!(
            "sessions.search",
            ctx,
            serde_json::json!({ "query": query })
        )
    }

    /// Resolve or auto-create a session.
    async fn resolve(&self, ctx: &Context<'_>, key: String) -> Result<Json> {
        rpc_query!("sessions.resolve", ctx, serde_json::json!({ "key": key }))
    }

    /// Get session branches.
    async fn branches(&self, ctx: &Context<'_>, key: Option<String>) -> Result<Json> {
        rpc_query!("sessions.branches", ctx, serde_json::json!({ "key": key }))
    }

    /// List shared session links.
    async fn shares(&self, ctx: &Context<'_>, key: Option<String>) -> Result<Json> {
        rpc_query!(
            "sessions.share.list",
            ctx,
            serde_json::json!({ "key": key })
        )
    }
}

#[derive(Default)]
pub struct ChannelQuery;

#[Object]
impl ChannelQuery {
    /// Get channel status.
    async fn status(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("channels.status", ctx)
    }

    /// List all channels.
    async fn list(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("channels.list", ctx)
    }

    /// List pending channel senders.
    async fn senders(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("channels.senders.list", ctx, serde_json::json!({}))
    }
}

#[derive(Default)]
pub struct ConfigQuery;

#[Object]
impl ConfigQuery {
    /// Get config value at a path.
    async fn get(&self, ctx: &Context<'_>, path: Option<String>) -> Result<Json> {
        rpc_query!("config.get", ctx, serde_json::json!({ "path": path }))
    }

    /// Get config schema definition.
    async fn schema(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("config.schema", ctx)
    }
}

#[derive(Default)]
pub struct CronQuery;

#[Object]
impl CronQuery {
    /// List all cron jobs.
    async fn list(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("cron.list", ctx)
    }

    /// Get cron status.
    async fn status(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("cron.status", ctx)
    }

    /// Get run history for a cron job.
    async fn runs(&self, ctx: &Context<'_>, job_id: String) -> Result<Json> {
        rpc_query!("cron.runs", ctx, serde_json::json!({ "jobId": job_id }))
    }
}

#[derive(Default)]
pub struct HeartbeatQuery;

#[Object]
impl HeartbeatQuery {
    /// Get heartbeat configuration and status.
    async fn status(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("heartbeat.status", ctx)
    }

    /// Get heartbeat run history.
    async fn runs(&self, ctx: &Context<'_>, limit: Option<u64>) -> Result<Json> {
        rpc_query!("heartbeat.runs", ctx, serde_json::json!({ "limit": limit }))
    }
}

#[derive(Default)]
pub struct LogsQuery;

#[Object]
impl LogsQuery {
    /// Stream log tail.
    async fn tail(&self, ctx: &Context<'_>, lines: Option<u64>) -> Result<Json> {
        rpc_query!("logs.tail", ctx, serde_json::json!({ "lines": lines }))
    }

    /// List logs.
    async fn list(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("logs.list", ctx)
    }

    /// Get log status.
    async fn status(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("logs.status", ctx)
    }
}

#[derive(Default)]
pub struct TtsQuery;

#[Object]
impl TtsQuery {
    /// Get TTS status.
    async fn status(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("tts.status", ctx)
    }

    /// Get available TTS providers.
    async fn providers(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("tts.providers", ctx)
    }

    /// Generate a TTS test phrase.
    async fn generate_phrase(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("tts.generate_phrase", ctx)
    }
}

#[derive(Default)]
pub struct SttQuery;

#[Object]
impl SttQuery {
    /// Get STT status.
    async fn status(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("stt.status", ctx)
    }

    /// Get available STT providers.
    async fn providers(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("stt.providers", ctx)
    }
}

#[derive(Default)]
pub struct VoiceQuery;

#[Object]
impl VoiceQuery {
    /// Get voice configuration.
    async fn config(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("voice.config.get", ctx)
    }

    /// Get all voice providers with availability detection.
    async fn providers(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("voice.providers.all", ctx)
    }

    /// Fetch ElevenLabs voice catalog.
    async fn elevenlabs_catalog(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("voice.elevenlabs.catalog", ctx)
    }

    /// Check Voxtral local setup requirements.
    async fn voxtral_requirements(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("voice.config.voxtral_requirements", ctx)
    }
}

#[derive(Default)]
pub struct SkillsQuery;

#[Object]
impl SkillsQuery {
    /// List installed skills.
    async fn list(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("skills.list", ctx)
    }

    /// Get skills system status.
    async fn status(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("skills.status", ctx)
    }

    /// Get skills binaries.
    async fn bins(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("skills.bins", ctx)
    }

    /// List skill repositories.
    async fn repos(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("skills.repos.list", ctx)
    }

    /// Get skill details.
    async fn detail(&self, ctx: &Context<'_>, name: String) -> Result<Json> {
        rpc_query!(
            "skills.skill.detail",
            ctx,
            serde_json::json!({ "name": name })
        )
    }

    /// Get security status.
    async fn security_status(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("skills.security.status", ctx)
    }

    /// Run security scan.
    async fn security_scan(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("skills.security.scan", ctx)
    }
}

#[derive(Default)]
pub struct ModelQuery;

#[Object]
impl ModelQuery {
    /// List enabled models.
    async fn list(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("models.list", ctx)
    }

    /// List all available models.
    async fn list_all(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("models.list_all", ctx)
    }
}

#[derive(Default)]
pub struct ProviderQuery;

#[Object]
impl ProviderQuery {
    /// List available provider integrations.
    async fn available(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("providers.available", ctx)
    }

    /// Get OAuth status.
    async fn oauth_status(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("providers.oauth.status", ctx)
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
        rpc_query!("providers.local.system_info", ctx)
    }

    /// List available local models.
    async fn models(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("providers.local.models", ctx)
    }

    /// Get local LLM status.
    async fn status(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("providers.local.status", ctx)
    }

    /// Search HuggingFace models.
    async fn search_hf(&self, ctx: &Context<'_>, query: String) -> Result<Json> {
        rpc_query!(
            "providers.local.search_hf",
            ctx,
            serde_json::json!({ "query": query })
        )
    }
}

#[derive(Default)]
pub struct McpQuery;

#[Object]
impl McpQuery {
    /// List MCP servers.
    async fn list(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("mcp.list", ctx)
    }

    /// Get MCP system status.
    async fn status(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("mcp.status", ctx, serde_json::json!({}))
    }

    /// Get MCP server tools.
    async fn tools(&self, ctx: &Context<'_>, name: Option<String>) -> Result<Json> {
        rpc_query!("mcp.tools", ctx, serde_json::json!({ "name": name }))
    }
}

#[derive(Default)]
pub struct UsageQuery;

#[Object]
impl UsageQuery {
    /// Get usage statistics.
    async fn status(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("usage.status", ctx)
    }

    /// Calculate cost for a usage period.
    async fn cost(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("usage.cost", ctx, serde_json::json!({}))
    }
}

#[derive(Default)]
pub struct ExecApprovalQuery;

#[Object]
impl ExecApprovalQuery {
    /// Get execution approval settings.
    async fn get(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("exec.approvals.get", ctx)
    }

    /// Get node-specific approval settings.
    async fn node_config(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("exec.approvals.node.get", ctx)
    }
}

#[derive(Default)]
pub struct ProjectQuery;

#[Object]
impl ProjectQuery {
    /// List all projects.
    async fn list(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("projects.list", ctx)
    }

    /// Get a project by ID.
    async fn get(&self, ctx: &Context<'_>, id: String) -> Result<Json> {
        rpc_query!("projects.get", ctx, serde_json::json!({ "id": id }))
    }

    /// Get project context.
    async fn context(&self, ctx: &Context<'_>, id: String) -> Result<Json> {
        rpc_query!("projects.context", ctx, serde_json::json!({ "id": id }))
    }

    /// Path completion for projects.
    async fn complete_path(&self, ctx: &Context<'_>, prefix: String) -> Result<Json> {
        rpc_query!(
            "projects.complete_path",
            ctx,
            serde_json::json!({ "prefix": prefix })
        )
    }
}

#[derive(Default)]
pub struct MemoryQuery;

#[Object]
impl MemoryQuery {
    /// Get memory system status.
    async fn status(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("memory.status", ctx)
    }

    /// Get memory configuration.
    async fn config(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("memory.config.get", ctx)
    }

    /// Get QMD status.
    async fn qmd_status(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("memory.qmd.status", ctx)
    }
}

#[derive(Default)]
pub struct HooksQuery;

#[Object]
impl HooksQuery {
    /// List discovered hooks with stats.
    async fn list(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("hooks.list", ctx)
    }
}

#[derive(Default)]
pub struct AgentQuery;

#[Object]
impl AgentQuery {
    /// List available agents.
    async fn list(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("agents.list", ctx)
    }

    /// Get agent identity.
    async fn identity(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("agent.identity.get", ctx)
    }
}

#[derive(Default)]
pub struct VoicewakeQuery;

#[Object]
impl VoicewakeQuery {
    /// Get wake word configuration.
    async fn get(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("voicewake.get", ctx)
    }
}

#[derive(Default)]
pub struct DeviceQuery;

#[Object]
impl DeviceQuery {
    /// List paired devices.
    async fn pair_requests(&self, ctx: &Context<'_>) -> Result<Json> {
        rpc_query!("device.pair.list", ctx)
    }
}
