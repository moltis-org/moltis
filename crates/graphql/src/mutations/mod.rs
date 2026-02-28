//! GraphQL mutation resolvers, organized by service namespace.
//!
//! Resolvers call domain services directly through the `Services` bundle —
//! no RPC string-based dispatch or `ServiceCaller` indirection.

use async_graphql::{Context, Object, Result};

use crate::{
    error::{from_service, from_service_json, gql_err, parse_err},
    scalars::Json,
    services,
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
        // System events are gateway-level; return ok.
        let _ = (ctx, event, payload);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    /// Touch activity timestamp.
    async fn set_heartbeats(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        let _ = ctx;
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    /// Trigger wake functionality.
    async fn wake(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.voicewake.wake(serde_json::json!({})).await)
    }

    /// Set talk mode.
    async fn talk_mode(&self, ctx: &Context<'_>, mode: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(
            s.voicewake
                .talk_mode(serde_json::json!({ "mode": mode }))
                .await,
        )
    }

    /// Check for and run updates.
    async fn update_run(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.update.run(serde_json::json!({})).await)
    }
}

// ── Node ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct NodeMutation;

#[Object]
impl NodeMutation {
    /// Forward RPC request to a node.
    async fn invoke(&self, ctx: &Context<'_>, input: Json) -> Result<Json> {
        // Node invoke is gateway-level; return placeholder.
        let _ = (ctx, input);
        from_service_json(Ok(serde_json::json!({ "ok": true })))
    }

    /// Rename a connected node.
    async fn rename(
        &self,
        ctx: &Context<'_>,
        node_id: String,
        display_name: String,
    ) -> Result<BoolResult> {
        let _ = (ctx, node_id, display_name);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    /// Request pairing with a new node.
    async fn pair_request(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let _ = (ctx, input);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    /// Approve node pairing.
    async fn pair_approve(&self, ctx: &Context<'_>, request_id: String) -> Result<BoolResult> {
        let _ = (ctx, request_id);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    /// Reject node pairing.
    async fn pair_reject(&self, ctx: &Context<'_>, request_id: String) -> Result<BoolResult> {
        let _ = (ctx, request_id);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    /// Verify node pairing signature.
    async fn pair_verify(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let _ = (ctx, input);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }
}

// ── Device ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct DeviceMutation;

#[Object]
impl DeviceMutation {
    async fn pair_approve(&self, ctx: &Context<'_>, device_id: String) -> Result<BoolResult> {
        let _ = (ctx, device_id);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    async fn pair_reject(&self, ctx: &Context<'_>, device_id: String) -> Result<BoolResult> {
        let _ = (ctx, device_id);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    async fn token_rotate(&self, ctx: &Context<'_>, device_id: String) -> Result<BoolResult> {
        let _ = (ctx, device_id);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    async fn token_revoke(&self, ctx: &Context<'_>, device_id: String) -> Result<BoolResult> {
        let _ = (ctx, device_id);
        from_service(Ok(serde_json::json!({ "ok": true })))
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
        let s = services!(ctx);
        from_service(
            s.chat
                .send(serde_json::json!({ "message": message, "sessionKey": session_key, "model": model }))
                .await,
        )
    }

    /// Abort active chat response.
    async fn abort(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(
            s.chat
                .abort(serde_json::json!({ "sessionKey": session_key }))
                .await,
        )
    }

    /// Cancel queued chat messages.
    async fn cancel_queued(
        &self,
        ctx: &Context<'_>,
        session_key: Option<String>,
    ) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(
            s.chat
                .cancel_queued(serde_json::json!({ "sessionKey": session_key }))
                .await,
        )
    }

    /// Clear chat history for session.
    async fn clear(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(
            s.chat
                .clear(serde_json::json!({ "sessionKey": session_key }))
                .await,
        )
    }

    /// Compact chat messages.
    async fn compact(&self, ctx: &Context<'_>, session_key: Option<String>) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(
            s.chat
                .compact(serde_json::json!({ "sessionKey": session_key }))
                .await,
        )
    }

    /// Inject a message into chat history.
    async fn inject(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.chat.inject(input.0).await)
    }
}

// ── Sessions ────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct SessionMutation;

#[Object]
impl SessionMutation {
    /// Switch active session.
    async fn switch(&self, ctx: &Context<'_>, key: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.session.resolve(serde_json::json!({ "key": key })).await)
    }

    /// Fork session to new session.
    async fn fork(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.session.fork(input.0).await)
    }

    /// Patch session metadata.
    async fn patch(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.session.patch(input.0).await)
    }

    /// Reset session history.
    async fn reset(&self, ctx: &Context<'_>, key: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.session.reset(serde_json::json!({ "key": key })).await)
    }

    /// Delete a session.
    async fn delete(&self, ctx: &Context<'_>, key: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.session.delete(serde_json::json!({ "key": key })).await)
    }

    /// Clear all sessions.
    async fn clear_all(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.session.clear_all().await)
    }

    /// Compact all sessions.
    async fn compact(&self, ctx: &Context<'_>, key: Option<String>) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.session.compact(serde_json::json!({ "key": key })).await)
    }

    /// Create a shareable session link.
    async fn share_create(&self, ctx: &Context<'_>, input: Json) -> Result<SessionShareResult> {
        let s = services!(ctx);
        from_service(s.session.share_create(input.0).await)
    }

    /// Revoke a shared session link.
    async fn share_revoke(&self, ctx: &Context<'_>, share_id: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(
            s.session
                .share_revoke(serde_json::json!({ "shareId": share_id }))
                .await,
        )
    }
}

// ── Channels ────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ChannelMutation;

#[Object]
impl ChannelMutation {
    async fn add(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.channel.add(input.0).await)
    }

    async fn remove(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.channel.remove(serde_json::json!({ "name": name })).await)
    }

    async fn update(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.channel.update(input.0).await)
    }

    async fn logout(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.channel.logout(serde_json::json!({ "name": name })).await)
    }

    async fn approve_sender(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.channel.sender_approve(input.0).await)
    }

    async fn deny_sender(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.channel.sender_deny(input.0).await)
    }
}

// ── Config ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ConfigMutation;

#[Object]
impl ConfigMutation {
    /// Set a config value.
    async fn set(&self, ctx: &Context<'_>, path: String, value: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(
            s.config
                .set(serde_json::json!({ "path": path, "value": value.0 }))
                .await,
        )
    }

    /// Apply full config.
    async fn apply(&self, ctx: &Context<'_>, config: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.config.apply(config.0).await)
    }

    /// Patch config.
    async fn patch(&self, ctx: &Context<'_>, patch: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.config.patch(patch.0).await)
    }
}

// ── Cron ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct CronMutation;

#[Object]
impl CronMutation {
    async fn add(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.cron.add(input.0).await)
    }

    async fn update(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.cron.update(input.0).await)
    }

    async fn remove(&self, ctx: &Context<'_>, id: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.cron.remove(serde_json::json!({ "id": id })).await)
    }

    /// Trigger a cron job immediately.
    async fn run(&self, ctx: &Context<'_>, id: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.cron.run(serde_json::json!({ "id": id })).await)
    }
}

// ── Heartbeat ───────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct HeartbeatMutation;

#[Object]
impl HeartbeatMutation {
    async fn update(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let _ = (ctx, input);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    async fn run(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        let _ = ctx;
        from_service(Ok(serde_json::json!({ "ok": true })))
    }
}

// ── TTS ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct TtsMutation;

#[Object]
impl TtsMutation {
    async fn enable(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.tts.enable(input.0).await)
    }

    async fn disable(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.tts.disable().await)
    }

    async fn convert(&self, ctx: &Context<'_>, audio: String) -> Result<TtsConvertResult> {
        let s = services!(ctx);
        from_service(s.tts.convert(serde_json::json!({ "audio": audio })).await)
    }

    async fn set_provider(&self, ctx: &Context<'_>, provider: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(
            s.tts
                .set_provider(serde_json::json!({ "provider": provider }))
                .await,
        )
    }
}

// ── STT ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct SttMutation;

#[Object]
impl SttMutation {
    async fn transcribe(&self, ctx: &Context<'_>, input: Json) -> Result<TranscriptionResult> {
        let s = services!(ctx);
        from_service(s.stt.transcribe(input.0).await)
    }

    async fn set_provider(&self, ctx: &Context<'_>, provider: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(
            s.stt
                .set_provider(serde_json::json!({ "provider": provider }))
                .await,
        )
    }
}

// ── Voice ───────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct VoiceMutation;

#[Object]
impl VoiceMutation {
    async fn save_key(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let _ = (ctx, input);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    async fn save_settings(&self, ctx: &Context<'_>, settings: Json) -> Result<BoolResult> {
        let _ = (ctx, settings);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    async fn remove_key(&self, ctx: &Context<'_>, provider: String) -> Result<BoolResult> {
        let _ = (ctx, provider);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    async fn toggle_provider(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let _ = (ctx, input);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    async fn session_override_set(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let _ = (ctx, input);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    async fn session_override_clear(
        &self,
        ctx: &Context<'_>,
        session_key: String,
    ) -> Result<BoolResult> {
        let _ = (ctx, session_key);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    async fn channel_override_set(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let _ = (ctx, input);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    async fn channel_override_clear(
        &self,
        ctx: &Context<'_>,
        channel_key: String,
    ) -> Result<BoolResult> {
        let _ = (ctx, channel_key);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }
}

// ── Skills ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct SkillsMutation;

#[Object]
impl SkillsMutation {
    async fn install(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.skills.install(input.0).await)
    }

    async fn remove(&self, ctx: &Context<'_>, source: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(
            s.skills
                .remove(serde_json::json!({ "source": source }))
                .await,
        )
    }

    async fn update(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.skills.update(serde_json::json!({ "name": name })).await)
    }

    async fn repos_remove(&self, ctx: &Context<'_>, source: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(
            s.skills
                .repos_remove(serde_json::json!({ "source": source }))
                .await,
        )
    }

    async fn emergency_disable(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.skills.emergency_disable().await)
    }

    async fn trust(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(
            s.skills
                .skill_trust(serde_json::json!({ "name": name }))
                .await,
        )
    }

    async fn enable(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(
            s.skills
                .skill_enable(serde_json::json!({ "name": name }))
                .await,
        )
    }

    async fn disable(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(
            s.skills
                .skill_disable(serde_json::json!({ "name": name }))
                .await,
        )
    }

    async fn install_dep(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.skills.install_dep(input.0).await)
    }
}

// ── Models ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ModelMutation;

#[Object]
impl ModelMutation {
    async fn enable(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.model.enable(input.0).await)
    }

    async fn disable(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.model.disable(input.0).await)
    }

    async fn detect_supported(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.model.detect_supported(serde_json::json!({})).await)
    }

    async fn test(&self, ctx: &Context<'_>, input: Json) -> Result<ModelTestResult> {
        let s = services!(ctx);
        from_service(s.model.test(input.0).await)
    }
}

// ── Providers ───────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ProviderMutation;

#[Object]
impl ProviderMutation {
    async fn save_key(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.provider_setup.save_key(input.0).await)
    }

    async fn validate_key(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.provider_setup.validate_key(input.0).await)
    }

    async fn save_model(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.provider_setup.save_model(input.0).await)
    }

    async fn save_models(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.provider_setup.save_models(input.0).await)
    }

    async fn remove_key(&self, ctx: &Context<'_>, provider: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(
            s.provider_setup
                .remove_key(serde_json::json!({ "provider": provider }))
                .await,
        )
    }

    async fn add_custom(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.provider_setup.add_custom(input.0).await)
    }

    async fn oauth_start(
        &self,
        ctx: &Context<'_>,
        provider: String,
    ) -> Result<ProviderOAuthStartResult> {
        let s = services!(ctx);
        from_service(
            s.provider_setup
                .oauth_start(serde_json::json!({ "provider": provider }))
                .await,
        )
    }

    async fn oauth_complete(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.provider_setup.oauth_complete(input.0).await)
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
        let s = services!(ctx);
        from_service(s.local_llm.configure(input.0).await)
    }

    async fn configure_custom(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.local_llm.configure_custom(input.0).await)
    }

    async fn remove_model(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.local_llm.remove_model(input.0).await)
    }
}

// ── MCP ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct McpMutation;

#[Object]
impl McpMutation {
    async fn add(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.mcp.add(input.0).await)
    }

    async fn remove(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.mcp.remove(serde_json::json!({ "name": name })).await)
    }

    async fn enable(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.mcp.enable(serde_json::json!({ "name": name })).await)
    }

    async fn disable(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.mcp.disable(serde_json::json!({ "name": name })).await)
    }

    async fn restart(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.mcp.restart(serde_json::json!({ "name": name })).await)
    }

    async fn reauth(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.mcp.reauth(serde_json::json!({ "name": name })).await)
    }

    async fn update(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.mcp.update(input.0).await)
    }

    async fn oauth_start(&self, ctx: &Context<'_>, name: String) -> Result<McpOAuthStartResult> {
        let s = services!(ctx);
        from_service(s.mcp.oauth_start(serde_json::json!({ "name": name })).await)
    }

    async fn oauth_complete(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.mcp.oauth_complete(input.0).await)
    }
}

// ── Projects ────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ProjectMutation;

#[Object]
impl ProjectMutation {
    async fn upsert(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.project.upsert(input.0).await)
    }

    async fn delete(&self, ctx: &Context<'_>, id: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.project.delete(serde_json::json!({ "id": id })).await)
    }

    async fn detect(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.project.detect(serde_json::json!({})).await)
    }
}

// ── Exec Approvals ──────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ExecApprovalMutation;

#[Object]
impl ExecApprovalMutation {
    async fn set(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.exec_approval.set(input.0).await)
    }

    async fn set_node_config(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.exec_approval.node_set(input.0).await)
    }

    async fn request(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.exec_approval.request(input.0).await)
    }

    async fn resolve(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.exec_approval.resolve(input.0).await)
    }
}

// ── Logs ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct LogsMutation;

#[Object]
impl LogsMutation {
    async fn ack(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.logs.ack().await)
    }
}

// ── Memory ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct MemoryMutation;

#[Object]
impl MemoryMutation {
    async fn update_config(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let _ = (ctx, input);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }
}

// ── Hooks ───────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct HooksMutation;

#[Object]
impl HooksMutation {
    async fn enable(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        let _ = (ctx, name);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    async fn disable(&self, ctx: &Context<'_>, name: String) -> Result<BoolResult> {
        let _ = (ctx, name);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    async fn save(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let _ = (ctx, input);
        from_service(Ok(serde_json::json!({ "ok": true })))
    }

    async fn reload(&self, ctx: &Context<'_>) -> Result<BoolResult> {
        let _ = ctx;
        from_service(Ok(serde_json::json!({ "ok": true })))
    }
}

// ── Agents ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct AgentMutation;

#[Object]
impl AgentMutation {
    /// Run agent with parameters.
    async fn run(&self, ctx: &Context<'_>, input: Json) -> Result<Json> {
        let s = services!(ctx);
        // Returns agent execution result with dynamic output.
        from_service_json(s.agent.run(input.0).await)
    }

    /// Run agent and wait for completion.
    async fn run_wait(&self, ctx: &Context<'_>, input: Json) -> Result<Json> {
        let s = services!(ctx);
        // Returns agent execution result with dynamic output.
        from_service_json(s.agent.run_wait(input.0).await)
    }

    /// Update agent identity.
    async fn update_identity(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);

        let payload = match input.0 {
            serde_json::Value::String(raw) => {
                serde_json::from_str::<serde_json::Value>(&raw).map_err(parse_err)?
            },
            value => value,
        };

        s.onboarding
            .identity_update(payload)
            .await
            .map_err(gql_err)?;
        Ok(BoolResult { ok: true })
    }

    /// Update agent soul/personality.
    async fn update_soul(&self, ctx: &Context<'_>, soul: String) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.onboarding.identity_update_soul(Some(soul)).await)
    }
}

// ── Voicewake ───────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct VoicewakeMutation;

#[Object]
impl VoicewakeMutation {
    async fn set(&self, ctx: &Context<'_>, input: Json) -> Result<BoolResult> {
        let s = services!(ctx);
        from_service(s.voicewake.set(input.0).await)
    }
}

// ── Browser ─────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct BrowserMutation;

#[Object]
impl BrowserMutation {
    async fn request(&self, ctx: &Context<'_>, input: Json) -> Result<Json> {
        let s = services!(ctx);
        // Returns browser response with dynamic content.
        from_service_json(s.browser.request(input.0).await)
    }
}
