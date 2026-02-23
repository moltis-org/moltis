//! GraphQL HTTP handlers for the gateway.
//!
//! These handlers bridge `AppState` to the `moltis-graphql` schema, providing
//! GraphiQL on GET `/graphql`, query/mutation execution on POST `/graphql`,
//! and WebSocket subscriptions on GET `/graphql`.

use std::sync::Arc;

use {
    async_graphql::http::GraphiQLSource,
    axum::{
        Json,
        extract::{FromRequestParts, Request, State, WebSocketUpgrade},
        http::{HeaderMap, StatusCode, header},
        response::{Html, IntoResponse, Response},
    },
    serde_json::Value,
};

use crate::{server::AppState, state::GatewayState};

/// `ServiceCaller` implementation that delegates to the gateway's `GatewayServices`.
///
/// This avoids exposing gateway internals to the graphql crate. Each RPC method
/// is dispatched to the corresponding service trait method.
pub struct GatewayServiceCaller {
    pub state: Arc<GatewayState>,
}

#[async_trait::async_trait]
impl moltis_graphql::context::ServiceCaller for GatewayServiceCaller {
    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        // Dispatch to the appropriate service based on method name.
        // This mirrors the gateway's MethodRegistry dispatch but goes through
        // the service trait objects directly.
        let s = &self.state.services;
        // Chat is late-bound via state.chat() so GraphQL uses the same LiveChatService as WebSocket/methods.
        let chat = self.state.chat().await;

        match method {
            // ── Health & Status ──────────────────────────────────────────
            "health" => Ok(
                serde_json::json!({ "ok": true, "connections": self.state.inner.read().await.clients.len() }),
            ),
            "status" => Ok(serde_json::json!({
                "hostname": self.state.hostname,
                "version": self.state.version,
                "connections": self.state.inner.read().await.clients.len(),
                "uptimeMs": self.state.uptime_ms(),
            })),
            "system-presence" => {
                let inner = self.state.inner.read().await;
                let clients: Vec<_> = inner
                    .clients
                    .values()
                    .map(|c| {
                        serde_json::json!({
                            "connId": c.conn_id,
                            "role": c.role(),
                            "connectedAt": c.connected_at.elapsed().as_secs(),
                        })
                    })
                    .collect();
                let nodes: Vec<_> = inner
                    .nodes
                    .list()
                    .iter()
                    .map(|n| {
                        serde_json::json!({
                            "nodeId": n.node_id,
                            "connId": n.conn_id,
                            "displayName": n.display_name,
                            "platform": n.platform,
                            "version": n.version,
                        })
                    })
                    .collect();
                Ok(serde_json::json!({ "clients": clients, "nodes": nodes }))
            },

            // ── Sessions ────────────────────────────────────────────────
            "sessions.list" => s.session.list().await,
            "sessions.preview" => s.session.preview(params).await,
            "sessions.search" => s.session.search(params).await,
            "sessions.resolve" => s.session.resolve(params).await,
            "sessions.patch" => s.session.patch(params).await,
            "sessions.reset" => s.session.reset(params).await,
            "sessions.delete" => s.session.delete(params).await,
            "sessions.clear_all" => s.session.clear_all().await,
            "sessions.compact" => s.session.compact(params).await,
            "sessions.fork" => s.session.fork(params).await,
            "sessions.branches" => s.session.branches(params).await,
            "sessions.share.create" => s.session.share_create(params).await,
            "sessions.share.list" => s.session.share_list(params).await,
            "sessions.share.revoke" => s.session.share_revoke(params).await,
            "sessions.switch" => {
                // Switch needs session resolve + mark_seen, simplified here.
                s.session.resolve(params).await
            },
            "sessions.active" => chat.active(params).await,

            // ── Chat ────────────────────────────────────────────────────
            "chat.send" => chat.send(params).await,
            "chat.abort" => chat.abort(params).await,
            "chat.cancel_queued" => chat.cancel_queued(params).await,
            "chat.history" => chat.history(params).await,
            "chat.inject" => chat.inject(params).await,
            "chat.clear" => chat.clear(params).await,
            "chat.compact" => chat.compact(params).await,
            "chat.context" => chat.context(params).await,
            "chat.raw_prompt" => chat.raw_prompt(params).await,
            "chat.full_context" => chat.full_context(params).await,

            // ── Config ──────────────────────────────────────────────────
            "config.get" => s.config.get(params).await,
            "config.set" => s.config.set(params).await,
            "config.apply" => s.config.apply(params).await,
            "config.patch" => s.config.patch(params).await,
            "config.schema" => s.config.schema().await,

            // ── Cron ────────────────────────────────────────────────────
            "cron.list" => s.cron.list().await,
            "cron.status" => s.cron.status().await,
            "cron.add" => s.cron.add(params).await,
            "cron.update" => s.cron.update(params).await,
            "cron.remove" => s.cron.remove(params).await,
            "cron.run" => s.cron.run(params).await,
            "cron.runs" => s.cron.runs(params).await,

            // ── TTS ─────────────────────────────────────────────────────
            "tts.status" => s.tts.status().await,
            "tts.providers" => s.tts.providers().await,
            "tts.enable" => s.tts.enable(params).await,
            "tts.disable" => s.tts.disable().await,
            "tts.convert" => s.tts.convert(params).await,
            "tts.setProvider" => s.tts.set_provider(params).await,

            // ── STT ─────────────────────────────────────────────────────
            "stt.status" => s.stt.status().await,
            "stt.providers" => s.stt.providers().await,
            "stt.transcribe" => s.stt.transcribe(params).await,
            "stt.setProvider" => s.stt.set_provider(params).await,

            // ── Skills ──────────────────────────────────────────────────
            "skills.list" => s.skills.list().await,
            "skills.status" => s.skills.status().await,
            "skills.bins" => s.skills.bins().await,
            "skills.install" => s.skills.install(params).await,
            "skills.update" => s.skills.update(params).await,
            "skills.remove" => s.skills.remove(params).await,
            "skills.repos.list" => s.skills.repos_list().await,
            "skills.repos.remove" => s.skills.repos_remove(params).await,
            "skills.emergency_disable" => s.skills.emergency_disable().await,
            "skills.skill.enable" => s.skills.skill_enable(params).await,
            "skills.skill.disable" => s.skills.skill_disable(params).await,
            "skills.skill.trust" => s.skills.skill_trust(params).await,
            "skills.skill.detail" => s.skills.skill_detail(params).await,
            "skills.install_dep" => s.skills.install_dep(params).await,
            "skills.security.status" => s.skills.security_status().await,
            "skills.security.scan" => s.skills.security_scan().await,

            // ── MCP ─────────────────────────────────────────────────────
            "mcp.list" => s.mcp.list().await,
            "mcp.add" => s.mcp.add(params).await,
            "mcp.remove" => s.mcp.remove(params).await,
            "mcp.enable" => s.mcp.enable(params).await,
            "mcp.disable" => s.mcp.disable(params).await,
            "mcp.status" => s.mcp.status(params).await,
            "mcp.tools" => s.mcp.tools(params).await,
            "mcp.restart" => s.mcp.restart(params).await,
            "mcp.update" => s.mcp.update(params).await,
            "mcp.reauth" => s.mcp.reauth(params).await,
            "mcp.oauth.start" => s.mcp.oauth_start(params).await,
            "mcp.oauth.complete" => s.mcp.oauth_complete(params).await,

            // ── Models ──────────────────────────────────────────────────
            "models.list" => s.model.list().await,
            "models.list_all" => s.model.list_all().await,
            "models.enable" => s.model.enable(params).await,
            "models.disable" => s.model.disable(params).await,
            "models.detect_supported" => s.model.detect_supported(params).await,
            "models.test" => s.model.test(params).await,

            // ── Providers ───────────────────────────────────────────────
            "providers.available" => s.provider_setup.available().await,
            "providers.save_key" => s.provider_setup.save_key(params).await,
            "providers.validate_key" => s.provider_setup.validate_key(params).await,
            "providers.save_model" => s.provider_setup.save_model(params).await,
            "providers.save_models" => s.provider_setup.save_models(params).await,
            "providers.remove_key" => s.provider_setup.remove_key(params).await,
            "providers.add_custom" => s.provider_setup.add_custom(params).await,
            "providers.oauth.start" => s.provider_setup.oauth_start(params).await,
            "providers.oauth.status" => s.provider_setup.oauth_status(params).await,
            "providers.oauth.complete" => s.provider_setup.oauth_complete(params).await,
            "providers.local.system_info" => s.local_llm.system_info().await,
            "providers.local.models" => s.local_llm.models().await,
            "providers.local.status" => s.local_llm.status().await,
            "providers.local.search_hf" => s.local_llm.search_hf(params).await,
            "providers.local.configure" => s.local_llm.configure(params).await,
            "providers.local.configure_custom" => s.local_llm.configure_custom(params).await,
            "providers.local.remove_model" => s.local_llm.remove_model(params).await,

            // ── Channels ────────────────────────────────────────────────
            "channels.status" => Ok(serde_json::json!({ "ok": true })),
            "channels.list" => {
                let result = s.channel.status().await?;
                Ok(result
                    .get("channels")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([])))
            },
            "channels.add" => s.channel.add(params).await,
            "channels.remove" => s.channel.remove(params).await,
            "channels.update" => s.channel.update(params).await,
            "channels.logout" => s.channel.logout(params).await,
            "channels.senders.list" => s.channel.senders_list(params).await,
            "channels.senders.approve" => s.channel.sender_approve(params).await,
            "channels.senders.deny" => s.channel.sender_deny(params).await,
            "send" => s.channel.send(params).await,

            // ── Usage ───────────────────────────────────────────────────
            "usage.status" => s.usage.status().await,
            "usage.cost" => s.usage.cost(params).await,

            // ── Exec Approvals ──────────────────────────────────────────
            "exec.approvals.get" => s.exec_approval.get().await,
            "exec.approvals.set" => s.exec_approval.set(params).await,
            "exec.approvals.node.get" => s.exec_approval.node_get(params).await,
            "exec.approvals.node.set" => s.exec_approval.node_set(params).await,
            "exec.approval.request" => s.exec_approval.request(params).await,
            "exec.approval.resolve" => s.exec_approval.resolve(params).await,

            // ── Projects ────────────────────────────────────────────────
            "projects.list" => s.project.list().await,
            "projects.get" => s.project.get(params).await,
            "projects.upsert" => s.project.upsert(params).await,
            "projects.delete" => s.project.delete(params).await,
            "projects.detect" => s.project.detect(params).await,
            "projects.context" => s.project.context(params).await,
            "projects.complete_path" => s.project.complete_path(params).await,

            // ── Logs ────────────────────────────────────────────────────
            "logs.tail" => s.logs.tail(params).await,
            "logs.list" => s.logs.list(params).await,
            "logs.status" => s.logs.status().await,
            "logs.ack" => s.logs.ack().await,

            // ── Memory ──────────────────────────────────────────────────
            "memory.status" => Ok(serde_json::json!({ "enabled": false })),
            "memory.config.get" => Ok(serde_json::json!({})),
            "memory.config.update" => Ok(serde_json::json!({ "ok": true })),
            "memory.qmd.status" => Ok(serde_json::json!({ "available": false })),

            // ── Agents ──────────────────────────────────────────────────
            "agent" => s.agent.run(params).await,
            "agent.wait" => s.agent.run_wait(params).await,
            "agent.identity.get" => s.agent.identity_get().await,
            "agent.identity.update" => Ok(serde_json::json!({ "ok": true })),
            "agent.identity.update_soul" => Ok(serde_json::json!({ "ok": true })),
            "agents.list" => s.agent.list().await,

            // ── Hooks ───────────────────────────────────────────────────
            "hooks.list" => {
                let inner = self.state.inner.read().await;
                let hooks: Vec<_> = inner
                    .discovered_hooks
                    .iter()
                    .map(|h| {
                        serde_json::json!({
                            "name": h.name,
                            "description": h.description,
                            "emoji": h.emoji,
                            "events": h.events,
                            "enabled": h.enabled,
                            "eligible": h.eligible,
                            "callCount": h.call_count,
                            "failureCount": h.failure_count,
                            "source": h.source,
                            "priority": h.priority,
                        })
                    })
                    .collect();
                Ok(serde_json::json!(hooks))
            },
            "hooks.enable" | "hooks.disable" | "hooks.save" | "hooks.reload" => {
                Ok(serde_json::json!({ "ok": true }))
            },

            // ── Heartbeat ───────────────────────────────────────────────
            "heartbeat.status" => {
                let inner = self.state.inner.read().await;
                Ok(serde_json::json!({ "config": inner.heartbeat_config }))
            },
            "heartbeat.update" | "heartbeat.run" => Ok(serde_json::json!({ "ok": true })),
            "heartbeat.runs" => Ok(serde_json::json!([])),

            // ── Voicewake ───────────────────────────────────────────────
            "voicewake.get" => s.voicewake.get().await,
            "voicewake.set" => s.voicewake.set(params).await,

            // ── Voice config ────────────────────────────────────────────
            "voice.config.get"
            | "voice.config.voxtral_requirements"
            | "voice.providers.all"
            | "voice.elevenlabs.catalog"
            | "voice.config.save_key"
            | "voice.config.save_settings"
            | "voice.config.remove_key"
            | "voice.provider.toggle"
            | "voice.override.session.set"
            | "voice.override.session.clear"
            | "voice.override.channel.set"
            | "voice.override.channel.clear" => {
                // Voice methods are handled by the VoiceConfigService in methods.rs.
                // For now, return a placeholder.
                Ok(serde_json::json!({ "ok": true }))
            },

            // ── TTS phrase generation ───────────────────────────────────
            "tts.generate_phrase" => Ok(serde_json::json!("Hello, how can I help you today?")),

            // ── Onboarding / Wizard ─────────────────────────────────────
            "wizard.start" => s.onboarding.wizard_start(params).await,
            "wizard.next" => s.onboarding.wizard_next(params).await,
            "wizard.cancel" => s.onboarding.wizard_cancel().await,
            "wizard.status" => Ok(serde_json::json!({ "ok": true })),

            // ── Web login ───────────────────────────────────────────────
            "web.login.start" => s.web_login.start(params).await,
            "web.login.wait" => s.web_login.wait(params).await,

            // ── Update ──────────────────────────────────────────────────
            "update.run" => s.update.run(params).await,

            // ── Browser ─────────────────────────────────────────────────
            "browser.request" => s.browser.request(params).await,

            // ── Device pairing ──────────────────────────────────────────
            "device.pair.list"
            | "device.pair.approve"
            | "device.pair.reject"
            | "device.token.rotate"
            | "device.token.revoke" => Ok(serde_json::json!({ "ok": true })),

            // ── Node methods ────────────────────────────────────────────
            "node.list" => {
                let inner = self.state.inner.read().await;
                let nodes: Vec<_> = inner
                    .nodes
                    .list()
                    .iter()
                    .map(|n| {
                        serde_json::json!({
                            "nodeId": n.node_id,
                            "connId": n.conn_id,
                            "displayName": n.display_name,
                            "platform": n.platform,
                            "version": n.version,
                        })
                    })
                    .collect();
                Ok(serde_json::json!(nodes))
            },
            "node.describe" | "node.rename" | "node.invoke" | "node.pair.request"
            | "node.pair.list" | "node.pair.approve" | "node.pair.reject" | "node.pair.verify" => {
                Ok(serde_json::json!({ "ok": true }))
            },

            // ── System misc ─────────────────────────────────────────────
            "last-heartbeat" | "set-heartbeats" | "system-event" | "wake" | "talk.mode"
            | "location.result" => Ok(serde_json::json!({ "ok": true })),

            _ => Err(format!("unknown method: {method}")),
        }
    }
}

/// Handle GET `/graphql`:
///
/// - Standard HTTP GET: returns GraphiQL.
/// - WebSocket upgrade GET: upgrades to GraphQL subscriptions.
pub async fn graphql_get_handler(State(state): State<AppState>, req: Request) -> impl IntoResponse {
    if !state.gateway.is_graphql_enabled() {
        return graphql_disabled_response();
    }

    let (mut parts, _body) = req.into_parts();

    if is_websocket_upgrade_request(&parts.headers) {
        let protocol =
            match async_graphql_axum::GraphQLProtocol::from_request_parts(&mut parts, &()).await {
                Ok(protocol) => protocol,
                Err(status) => return status.into_response(),
            };

        let ws = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
            Ok(ws) => ws,
            Err(rejection) => return rejection.into_response(),
        };

        return graphql_ws_response(&state, protocol, ws);
    }

    graphiql_response()
}

/// Handle GraphQL queries and mutations.
pub async fn graphql_handler(
    State(state): State<AppState>,
    req: async_graphql_axum::GraphQLRequest,
) -> impl IntoResponse {
    if !state.gateway.is_graphql_enabled() {
        return graphql_disabled_response();
    }

    async_graphql_axum::GraphQLResponse::from(state.graphql_schema.execute(req.into_inner()).await)
        .into_response()
}

fn graphql_ws_response(
    state: &AppState,
    protocol: async_graphql_axum::GraphQLProtocol,
    ws: WebSocketUpgrade,
) -> Response {
    let schema = state.graphql_schema.clone();
    ws.protocols(["graphql-transport-ws", "graphql-ws"])
        .on_upgrade(move |socket| {
            let resp = async_graphql_axum::GraphQLWebSocket::new(socket, schema, protocol);
            async move {
                resp.serve().await;
            }
        })
        .into_response()
}

fn graphiql_response() -> Response {
    Html(
        GraphiQLSource::build()
            .endpoint("/graphql")
            .subscription_endpoint("/graphql")
            .finish(),
    )
    .into_response()
}

fn graphql_disabled_response() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({ "error": "graphql server is disabled" })),
    )
        .into_response()
}

fn is_websocket_upgrade_request(headers: &HeaderMap) -> bool {
    // A proper WS upgrade has Connection: Upgrade AND Upgrade: websocket,
    // but we also accept the presence of Sec-WebSocket-Key as a fallback
    // since some clients (e.g. graphql-ws) may omit the Connection header.
    let has_upgrade_header = headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(',')
                .any(|t| t.trim().eq_ignore_ascii_case("websocket"))
        })
        .unwrap_or(false);

    has_upgrade_header || headers.contains_key(header::SEC_WEBSOCKET_KEY)
}
