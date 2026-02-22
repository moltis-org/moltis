//! GraphQL mutation resolvers, organized by RPC namespace.

use async_graphql::{Context, Object, Result};

use crate::{
    rpc_call, rpc_json_call,
    scalars::Json,
    types::{
        BoolResult, McpOAuthStartResult, ModelTestResult, ProviderOAuthStartResult,
        SessionShareResult, TranscriptionResult, TtsConvertResult,
    },
};

/// Root mutation type composing all namespace mutations.
#[derive(Default)]
pub struct MutationRoot;

#[Object]
impl MutationRoot {
    async fn system(&self) -> SystemMutation {
        SystemMutation
    }

    async fn node(&self) -> NodeMutation {
        NodeMutation
    }

    async fn device(&self) -> DeviceMutation {
        DeviceMutation
    }

    async fn chat(&self) -> ChatMutation {
        ChatMutation
    }

    async fn sessions(&self) -> SessionMutation {
        SessionMutation
    }

    async fn channels(&self) -> ChannelMutation {
        ChannelMutation
    }

    async fn config(&self) -> ConfigMutation {
        ConfigMutation
    }

    async fn cron(&self) -> CronMutation {
        CronMutation
    }

    async fn heartbeat(&self) -> HeartbeatMutation {
        HeartbeatMutation
    }

    async fn tts(&self) -> TtsMutation {
        TtsMutation
    }

    async fn stt(&self) -> SttMutation {
        SttMutation
    }

    async fn voice(&self) -> VoiceMutation {
        VoiceMutation
    }

    async fn skills(&self) -> SkillsMutation {
        SkillsMutation
    }

    async fn models(&self) -> ModelMutation {
        ModelMutation
    }

    async fn providers(&self) -> ProviderMutation {
        ProviderMutation
    }

    async fn mcp(&self) -> McpMutation {
        McpMutation
    }

    async fn projects(&self) -> ProjectMutation {
        ProjectMutation
    }

    async fn exec_approvals(&self) -> ExecApprovalMutation {
        ExecApprovalMutation
    }

    async fn logs(&self) -> LogsMutation {
        LogsMutation
    }

    async fn memory(&self) -> MemoryMutation {
        MemoryMutation
    }

    async fn hooks(&self) -> HooksMutation {
        HooksMutation
    }

    async fn agents(&self) -> AgentMutation {
        AgentMutation
    }

    async fn voicewake(&self) -> VoicewakeMutation {
        VoicewakeMutation
    }

    async fn browser(&self) -> BrowserMutation {
        BrowserMutation
    }
}

// ── System ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct SystemMutation;

#[Object]
impl SystemMutation {
    /// Broadcast a system event.
    async fn event(
        &self,
        ctx: &Context<'_>,
        event: String,
        payload: Option<Json>,
    ) -> Result<BoolResult> {
        rpc_call!(
            "system-event",
            ctx,
            serde_json::json!({ "event": event, "payload": payload.map(|p| p.0) })
        )
    }

    /// Touch activity timestamp.
    async fn set_heartbeats(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_call!("set-heartbeats", ctx)
    }

    /// Trigger wake functionality.
    async fn wake(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_call!("wake", ctx)
    }

    /// Set talk mode.
    async fn talk_mode(&self, ctx: &Context<'_>, mode: String) -> Result<BoolResult> {
        rpc_call!("talk.mode", ctx, serde_json::json!({ "mode": mode }))
    }

    /// Check for and run updates.
    async fn update_run(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_call!("update.run", ctx)
    }
}

// ── Node ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct NodeMutation;

#[Object]
impl NodeMutation {
    /// Forward RPC request to a node.
    async fn invoke(&self, ctx: &Context<'_>, input: Json) -> Result<Json> {
        // Node invoke returns dynamic response from the target node.
        rpc_json_call!("node.invoke", ctx, input.0)
    }

    /// Rename a connected node.
    async fn rename(
        &self,
        ctx: &Context<'_>,
        node_id: String,
        display_name: String,
    ) -> Result<BoolResult> {
        rpc_call!(
            "node.rename",
            ctx,
            serde_json::json!({ "nodeId": node_id, "displayName": display_name })
        )
    }

    /// Request pairing with a new node.
    async fn pair_request(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("node.pair.request", ctx, input.0)
    }

    /// Approve node pairing.
    async fn pair_approve(&self, ctx: &Context<'_>, request_id: String) -> Result<BoolResult> {
        rpc_call!(
            "node.pair.approve",
            ctx,
            serde_json::json!({ "requestId": request_id })
        )
    }

    /// Reject node pairing.
    async fn pair_reject(&self, ctx: &Context<'_>, request_id: String) -> Result<BoolResult> {
        rpc_call!(
            "node.pair.reject",
            ctx,
            serde_json::json!({ "requestId": request_id })
        )
    }

    /// Verify node pairing signature.
    async fn pair_verify(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("node.pair.verify", ctx, input.0)
    }
}

// ── Device ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct DeviceMutation;

#[Object]
impl DeviceMutation {
    async fn pair_approve(&self, ctx: &Context<'_>, device_id: String) -> Result<BoolResult> {
        rpc_call!(
            "device.pair.approve",
            ctx,
            serde_json::json!({ "deviceId": device_id })
        )
    }

    async fn pair_reject(&self, ctx: &Context<'_>, device_id: String) -> Result<BoolResult> {
        rpc_call!(
            "device.pair.reject",
            ctx,
            serde_json::json!({ "deviceId": device_id })
        )
    }

    async fn token_rotate(&self, ctx: &Context<'_>, device_id: String) -> Result<BoolResult> {
        rpc_call!(
            "device.token.rotate",
            ctx,
            serde_json::json!({ "deviceId": device_id })
        )
    }

    async fn token_revoke(&self, ctx: &Context<'_>, device_id: String) -> Result<BoolResult> {
        rpc_call!(
            "device.token.revoke",
            ctx,
            serde_json::json!({ "deviceId": device_id })
        )
    }
}

// ── Chat ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ChatMutation;

#[Object]
impl ChatMutation {
    /// Send a chat message.
    async fn send(
        &self,
        ctx: &Context<'_>,
        message: String,
        session_key: Option<String>,
        model: Option<String>,
    ) -> Result<BoolResult> {
        rpc_call!(
            "chat.send",
            ctx,
            serde_json::json!({ "message": message, "sessionKey": session_key, "model": model })
        )
    }

    /// Abort active chat response.
    async fn abort(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<BoolResult> {
        rpc_call!(
            "chat.abort",
            ctx,
            serde_json::json!({ "sessionKey": session_key })
        )
    }

    /// Cancel queued chat messages.
    async fn cancel_queued(
        &self,
        ctx: &Context<'_>,
        session_key: Option<String>,
    ) -> Result<BoolResult> {
        rpc_call!(
            "chat.cancel_queued",
            ctx,
            serde_json::json!({ "sessionKey": session_key })
        )
    }

    /// Clear chat history for session.
    async fn clear(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<BoolResult> {
        rpc_call!(
            "chat.clear",
            ctx,
            serde_json::json!({ "sessionKey": session_key })
        )
    }

    /// Compact chat messages.
    async fn compact(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<BoolResult> {
        rpc_call!(
            "chat.compact",
            ctx,
            serde_json::json!({ "sessionKey": session_key })
        )
    }

    /// Inject a message into chat history.
    async fn inject(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("chat.inject", ctx, input.0)
    }
}

// ── Sessions ────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct SessionMutation;

#[Object]
impl SessionMutation {
    /// Switch active session.
    async fn switch(&self, ctx: &Context<'_>, key: String) -> Result<BoolResult> {
        rpc_call!("sessions.switch", ctx, serde_json::json!({ "key": key }))
    }

    /// Fork session to new session.
    async fn fork(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("sessions.fork", ctx, input.0)
    }

    /// Patch session metadata.
    async fn patch(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("sessions.patch", ctx, input.0)
    }

    /// Reset session history.
    async fn reset(&self, ctx: &Context<'_>, key: String) -> Result<BoolResult> {
        rpc_call!("sessions.reset", ctx, serde_json::json!({ "key": key }))
    }

    /// Delete a session.
    async fn delete(&self, ctx: &Context<'_>, key: String) -> Result<BoolResult> {
        rpc_call!("sessions.delete", ctx, serde_json::json!({ "key": key }))
    }

    /// Clear all sessions.
    async fn clear_all(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_call!("sessions.clear_all", ctx)
    }

    /// Compact all sessions.
    async fn compact(&self, ctx: &Context<'_>, key: Option<String>) -> Result<BoolResult> {
        rpc_call!("sessions.compact", ctx, serde_json::json!({ "key": key }))
    }

    /// Create a shareable session link.
    async fn share_create(&self, ctx: &Context<'_>, input: Json) -> Result<SessionShareResult> {
        rpc_call!("sessions.share.create", ctx, input.0)
    }

    /// Revoke a shared session link.
    async fn share_revoke(&self, ctx: &Context<'_>, share_id: String) -> Result<BoolResult> {
        rpc_call!(
            "sessions.share.revoke",
            ctx,
            serde_json::json!({ "shareId": share_id })
        )
    }
}

// ── Channels ────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ChannelMutation;

#[Object]
impl ChannelMutation {
    async fn add(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("channels.add", ctx, input.0)
    }

    async fn remove(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        rpc_call!("channels.remove", ctx, serde_json::json!({ "name": name }))
    }

    async fn update(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("channels.update", ctx, input.0)
    }

    async fn logout(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        rpc_call!("channels.logout", ctx, serde_json::json!({ "name": name }))
    }

    async fn approve_sender(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("channels.senders.approve", ctx, input.0)
    }

    async fn deny_sender(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("channels.senders.deny", ctx, input.0)
    }
}

// ── Config ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ConfigMutation;

#[Object]
impl ConfigMutation {
    /// Set a config value.
    async fn set(&self, ctx: &Context<'_>, path: String, value: Json) -> Result<BoolResult> {
        rpc_call!(
            "config.set",
            ctx,
            serde_json::json!({ "path": path, "value": value.0 })
        )
    }

    /// Apply full config.
    async fn apply(&self, ctx: &Context<'_>, config: Json) -> Result<BoolResult> {
        rpc_call!("config.apply", ctx, config.0)
    }

    /// Patch config.
    async fn patch(&self, ctx: &Context<'_>, patch: Json) -> Result<BoolResult> {
        rpc_call!("config.patch", ctx, patch.0)
    }
}

// ── Cron ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct CronMutation;

#[Object]
impl CronMutation {
    async fn add(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("cron.add", ctx, input.0)
    }

    async fn update(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("cron.update", ctx, input.0)
    }

    async fn remove(&self, ctx: &Context<'_>, id: String) -> Result<BoolResult> {
        rpc_call!("cron.remove", ctx, serde_json::json!({ "id": id }))
    }

    /// Trigger a cron job immediately.
    async fn run(&self, ctx: &Context<'_>, id: String) -> Result<BoolResult> {
        rpc_call!("cron.run", ctx, serde_json::json!({ "id": id }))
    }
}

// ── Heartbeat ───────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct HeartbeatMutation;

#[Object]
impl HeartbeatMutation {
    async fn update(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("heartbeat.update", ctx, input.0)
    }

    async fn run(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_call!("heartbeat.run", ctx)
    }
}

// ── TTS ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct TtsMutation;

#[Object]
impl TtsMutation {
    async fn enable(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("tts.enable", ctx, input.0)
    }

    async fn disable(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_call!("tts.disable", ctx)
    }

    async fn convert(&self, ctx: &Context<'_>, audio: String) -> Result<TtsConvertResult> {
        rpc_call!("tts.convert", ctx, serde_json::json!({ "audio": audio }))
    }

    async fn set_provider(&self, ctx: &Context<'_>, provider: String) -> Result<BoolResult> {
        rpc_call!(
            "tts.setProvider",
            ctx,
            serde_json::json!({ "provider": provider })
        )
    }
}

// ── STT ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct SttMutation;

#[Object]
impl SttMutation {
    async fn transcribe(&self, ctx: &Context<'_>, input: Json) -> Result<TranscriptionResult> {
        rpc_call!("stt.transcribe", ctx, input.0)
    }

    async fn set_provider(&self, ctx: &Context<'_>, provider: String) -> Result<BoolResult> {
        rpc_call!(
            "stt.setProvider",
            ctx,
            serde_json::json!({ "provider": provider })
        )
    }
}

// ── Voice ───────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct VoiceMutation;

#[Object]
impl VoiceMutation {
    async fn save_key(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("voice.config.save_key", ctx, input.0)
    }

    async fn save_settings(&self, ctx: &Context<'_>, settings: Json) -> Result<BoolResult> {
        rpc_call!("voice.config.save_settings", ctx, settings.0)
    }

    async fn remove_key(&self, ctx: &Context<'_>, provider: String) -> Result<BoolResult> {
        rpc_call!(
            "voice.config.remove_key",
            ctx,
            serde_json::json!({ "provider": provider })
        )
    }

    async fn toggle_provider(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("voice.provider.toggle", ctx, input.0)
    }

    async fn session_override_set(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("voice.override.session.set", ctx, input.0)
    }

    async fn session_override_clear(
        &self,
        ctx: &Context<'_>,
        session_key: String,
    ) -> Result<BoolResult> {
        rpc_call!(
            "voice.override.session.clear",
            ctx,
            serde_json::json!({ "sessionKey": session_key })
        )
    }

    async fn channel_override_set(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("voice.override.channel.set", ctx, input.0)
    }

    async fn channel_override_clear(
        &self,
        ctx: &Context<'_>,
        channel_key: String,
    ) -> Result<BoolResult> {
        rpc_call!(
            "voice.override.channel.clear",
            ctx,
            serde_json::json!({ "channelKey": channel_key })
        )
    }
}

// ── Skills ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct SkillsMutation;

#[Object]
impl SkillsMutation {
    async fn install(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("skills.install", ctx, input.0)
    }

    async fn remove(&self, ctx: &Context<'_>, source: String) -> Result<BoolResult> {
        rpc_call!(
            "skills.remove",
            ctx,
            serde_json::json!({ "source": source })
        )
    }

    async fn update(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        rpc_call!("skills.update", ctx, serde_json::json!({ "name": name }))
    }

    async fn repos_remove(&self, ctx: &Context<'_>, source: String) -> Result<BoolResult> {
        rpc_call!(
            "skills.repos.remove",
            ctx,
            serde_json::json!({ "source": source })
        )
    }

    async fn emergency_disable(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_call!("skills.emergency_disable", ctx)
    }

    async fn trust(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        rpc_call!(
            "skills.skill.trust",
            ctx,
            serde_json::json!({ "name": name })
        )
    }

    async fn enable(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        rpc_call!(
            "skills.skill.enable",
            ctx,
            serde_json::json!({ "name": name })
        )
    }

    async fn disable(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        rpc_call!(
            "skills.skill.disable",
            ctx,
            serde_json::json!({ "name": name })
        )
    }

    async fn install_dep(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("skills.install_dep", ctx, input.0)
    }
}

// ── Models ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ModelMutation;

#[Object]
impl ModelMutation {
    async fn enable(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("models.enable", ctx, input.0)
    }

    async fn disable(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("models.disable", ctx, input.0)
    }

    async fn detect_supported(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_call!("models.detect_supported", ctx)
    }

    async fn test(&self, ctx: &Context<'_>, input: Json) -> Result<ModelTestResult> {
        rpc_call!("models.test", ctx, input.0)
    }
}

// ── Providers ───────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ProviderMutation;

#[Object]
impl ProviderMutation {
    async fn save_key(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("providers.save_key", ctx, input.0)
    }

    async fn validate_key(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("providers.validate_key", ctx, input.0)
    }

    async fn save_model(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("providers.save_model", ctx, input.0)
    }

    async fn save_models(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("providers.save_models", ctx, input.0)
    }

    async fn remove_key(&self, ctx: &Context<'_>, provider: String) -> Result<BoolResult> {
        rpc_call!(
            "providers.remove_key",
            ctx,
            serde_json::json!({ "provider": provider })
        )
    }

    async fn add_custom(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("providers.add_custom", ctx, input.0)
    }

    async fn oauth_start(
        &self,
        ctx: &Context<'_>,
        provider: String,
    ) -> Result<ProviderOAuthStartResult> {
        rpc_call!(
            "providers.oauth.start",
            ctx,
            serde_json::json!({ "provider": provider })
        )
    }

    async fn oauth_complete(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("providers.oauth.complete", ctx, input.0)
    }

    /// Local LLM mutations.
    async fn local(&self) -> LocalLlmMutation {
        LocalLlmMutation
    }
}

#[derive(Default)]
pub struct LocalLlmMutation;

#[Object]
impl LocalLlmMutation {
    async fn configure(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("providers.local.configure", ctx, input.0)
    }

    async fn configure_custom(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("providers.local.configure_custom", ctx, input.0)
    }

    async fn remove_model(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("providers.local.remove_model", ctx, input.0)
    }
}

// ── MCP ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct McpMutation;

#[Object]
impl McpMutation {
    async fn add(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("mcp.add", ctx, input.0)
    }

    async fn remove(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        rpc_call!("mcp.remove", ctx, serde_json::json!({ "name": name }))
    }

    async fn enable(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        rpc_call!("mcp.enable", ctx, serde_json::json!({ "name": name }))
    }

    async fn disable(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        rpc_call!("mcp.disable", ctx, serde_json::json!({ "name": name }))
    }

    async fn restart(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        rpc_call!("mcp.restart", ctx, serde_json::json!({ "name": name }))
    }

    async fn reauth(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        rpc_call!("mcp.reauth", ctx, serde_json::json!({ "name": name }))
    }

    async fn update(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("mcp.update", ctx, input.0)
    }

    async fn oauth_start(&self, ctx: &Context<'_>, name: String) -> Result<McpOAuthStartResult> {
        rpc_call!("mcp.oauth.start", ctx, serde_json::json!({ "name": name }))
    }

    async fn oauth_complete(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("mcp.oauth.complete", ctx, input.0)
    }
}

// ── Projects ────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ProjectMutation;

#[Object]
impl ProjectMutation {
    async fn upsert(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("projects.upsert", ctx, input.0)
    }

    async fn delete(&self, ctx: &Context<'_>, id: String) -> Result<BoolResult> {
        rpc_call!("projects.delete", ctx, serde_json::json!({ "id": id }))
    }

    async fn detect(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_call!("projects.detect", ctx)
    }
}

// ── Exec Approvals ──────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ExecApprovalMutation;

#[Object]
impl ExecApprovalMutation {
    async fn set(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("exec.approvals.set", ctx, input.0)
    }

    async fn set_node_config(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("exec.approvals.node.set", ctx, input.0)
    }

    async fn request(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("exec.approval.request", ctx, input.0)
    }

    async fn resolve(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("exec.approval.resolve", ctx, input.0)
    }
}

// ── Logs ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct LogsMutation;

#[Object]
impl LogsMutation {
    async fn ack(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_call!("logs.ack", ctx)
    }
}

// ── Memory ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct MemoryMutation;

#[Object]
impl MemoryMutation {
    async fn update_config(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("memory.config.update", ctx, input.0)
    }
}

// ── Hooks ───────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct HooksMutation;

#[Object]
impl HooksMutation {
    async fn enable(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        rpc_call!("hooks.enable", ctx, serde_json::json!({ "name": name }))
    }

    async fn disable(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        rpc_call!("hooks.disable", ctx, serde_json::json!({ "name": name }))
    }

    async fn save(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("hooks.save", ctx, input.0)
    }

    async fn reload(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        rpc_call!("hooks.reload", ctx)
    }
}

// ── Agents ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct AgentMutation;

#[Object]
impl AgentMutation {
    /// Run agent with parameters.
    async fn run(&self, ctx: &Context<'_>, input: Json) -> Result<Json> {
        // Returns agent execution result with dynamic output.
        rpc_json_call!("agent", ctx, input.0)
    }

    /// Run agent and wait for completion.
    async fn run_wait(&self, ctx: &Context<'_>, input: Json) -> Result<Json> {
        // Returns agent execution result with dynamic output.
        rpc_json_call!("agent.wait", ctx, input.0)
    }

    /// Update agent identity.
    async fn update_identity(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("agent.identity.update", ctx, input.0)
    }

    /// Update agent soul/personality.
    async fn update_soul(&self, ctx: &Context<'_>, soul: String) -> Result<BoolResult> {
        rpc_call!(
            "agent.identity.update_soul",
            ctx,
            serde_json::json!({ "soul": soul })
        )
    }
}

// ── Voicewake ───────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct VoicewakeMutation;

#[Object]
impl VoicewakeMutation {
    async fn set(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        rpc_call!("voicewake.set", ctx, input.0)
    }
}

// ── Browser ─────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct BrowserMutation;

#[Object]
impl BrowserMutation {
    async fn request(&self, ctx: &Context<'_>, input: Json) -> Result<Json> {
        // Returns browser response with dynamic content.
        rpc_json_call!("browser.request", ctx, input.0)
    }
}
