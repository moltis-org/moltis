use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use tracing::{debug, warn};

use {
    moltis_config::VoiceSttProvider,
    moltis_protocol::{ErrorShape, ResponseFrame, error_codes},
};

use crate::{
    broadcast::{BroadcastOpts, broadcast},
    state::GatewayState,
};

// ── Types ────────────────────────────────────────────────────────────────────

/// Context passed to every method handler.
pub struct MethodContext {
    pub request_id: String,
    pub method: String,
    pub params: serde_json::Value,
    pub client_conn_id: String,
    pub client_role: String,
    pub client_scopes: Vec<String>,
    pub state: Arc<GatewayState>,
}

/// The result a method handler produces.
pub type MethodResult = Result<serde_json::Value, ErrorShape>;

/// A boxed async method handler.
pub type HandlerFn =
    Box<dyn Fn(MethodContext) -> Pin<Box<dyn Future<Output = MethodResult> + Send>> + Send + Sync>;

// ── Scope authorization ──────────────────────────────────────────────────────

const NODE_METHODS: &[&str] = &["node.invoke.result", "node.event", "skills.bins"];

const READ_METHODS: &[&str] = &[
    "health",
    "logs.tail",
    "logs.list",
    "logs.status",
    "channels.status",
    "channels.list",
    "channels.senders.list",
    "status",
    "usage.status",
    "usage.cost",
    "tts.status",
    "tts.providers",
    "stt.status",
    "stt.providers",
    "models.list",
    "models.list_all",
    "agents.list",
    "agent.identity.get",
    "skills.list",
    "skills.status",
    "skills.security.status",
    "skills.repos.list",
    "skills.security.scan",
    "voicewake.get",
    "sessions.list",
    "sessions.preview",
    "sessions.search",
    "sessions.branches",
    "projects.list",
    "projects.get",
    "projects.context",
    "projects.complete_path",
    "cron.list",
    "cron.status",
    "cron.runs",
    "heartbeat.status",
    "heartbeat.runs",
    "system-presence",
    "last-heartbeat",
    "node.list",
    "node.describe",
    "chat.history",
    "chat.context",
    "providers.available",
    "providers.oauth.status",
    "providers.local.system_info",
    "providers.local.models",
    "providers.local.status",
    "providers.local.search_hf",
    "mcp.list",
    "mcp.status",
    "mcp.tools",
    "voice.config.get",
    "voice.config.voxtral_requirements",
    "voice.providers.all",
    "voice.elevenlabs.catalog",
    "memory.status",
    "memory.config.get",
    "memory.qmd.status",
    "hooks.list",
];

const WRITE_METHODS: &[&str] = &[
    "send",
    "agent",
    "agent.wait",
    "agent.identity.update",
    "agent.identity.update_soul",
    "wake",
    "talk.mode",
    "tts.enable",
    "tts.disable",
    "tts.convert",
    "tts.setProvider",
    "stt.transcribe",
    "stt.setProvider",
    "voicewake.set",
    "node.invoke",
    "chat.send",
    "chat.abort",
    "chat.clear",
    "chat.compact",
    "browser.request",
    "logs.ack",
    "models.detect_supported",
    "providers.save_key",
    "providers.remove_key",
    "providers.oauth.start",
    "providers.oauth.complete",
    "providers.local.configure",
    "providers.local.configure_custom",
    "channels.add",
    "channels.remove",
    "channels.update",
    "channels.senders.approve",
    "channels.senders.deny",
    "sessions.switch",
    "sessions.fork",
    "projects.upsert",
    "projects.delete",
    "projects.detect",
    "skills.install",
    "skills.remove",
    "skills.repos.remove",
    "skills.emergency_disable",
    "skills.skill.trust",
    "skills.skill.enable",
    "skills.skill.disable",
    "skills.install_dep",
    "mcp.add",
    "mcp.remove",
    "mcp.enable",
    "mcp.disable",
    "mcp.restart",
    "mcp.update",
    "cron.add",
    "cron.update",
    "cron.remove",
    "cron.run",
    "heartbeat.update",
    "heartbeat.run",
    "voice.config.save_key",
    "voice.config.save_settings",
    "voice.config.remove_key",
    "voice.provider.toggle",
    "voice.override.session.set",
    "voice.override.session.clear",
    "voice.override.channel.set",
    "voice.override.channel.clear",
    "memory.config.update",
    "hooks.enable",
    "hooks.disable",
    "hooks.save",
    "hooks.reload",
];

const APPROVAL_METHODS: &[&str] = &["exec.approval.request", "exec.approval.resolve"];

const PAIRING_METHODS: &[&str] = &[
    "node.pair.request",
    "node.pair.list",
    "node.pair.approve",
    "node.pair.reject",
    "node.pair.verify",
    "device.pair.list",
    "device.pair.approve",
    "device.pair.reject",
    "device.token.rotate",
    "device.token.revoke",
    "node.rename",
];

fn is_in(method: &str, list: &[&str]) -> bool {
    list.contains(&method)
}

fn model_probe_params(provider: Option<&str>) -> serde_json::Value {
    let mut params = serde_json::json!({
        "background": true,
        "reason": "provider_connected",
    });
    if let Some(provider) = provider
        && !provider.trim().is_empty()
    {
        params["provider"] = serde_json::json!(provider);
    }
    params
}

/// Check role + scopes for a method. Returns None if authorized, Some(error) if not.
pub fn authorize_method(method: &str, role: &str, scopes: &[String]) -> Option<ErrorShape> {
    use moltis_protocol::scopes as s;

    if is_in(method, NODE_METHODS) {
        if role == "node" {
            return None;
        }
        return Some(ErrorShape::new(
            error_codes::INVALID_REQUEST,
            format!("unauthorized role: {role}"),
        ));
    }
    if role == "node" || role != "operator" {
        return Some(ErrorShape::new(
            error_codes::INVALID_REQUEST,
            format!("unauthorized role: {role}"),
        ));
    }

    let has = |scope: &str| scopes.iter().any(|s| s == scope);
    if has(s::ADMIN) {
        return None;
    }

    if is_in(method, APPROVAL_METHODS) && !has(s::APPROVALS) {
        return Some(ErrorShape::new(
            error_codes::INVALID_REQUEST,
            "missing scope: operator.approvals",
        ));
    }
    if is_in(method, PAIRING_METHODS) && !has(s::PAIRING) {
        return Some(ErrorShape::new(
            error_codes::INVALID_REQUEST,
            "missing scope: operator.pairing",
        ));
    }
    if is_in(method, READ_METHODS) && !(has(s::READ) || has(s::WRITE)) {
        return Some(ErrorShape::new(
            error_codes::INVALID_REQUEST,
            "missing scope: operator.read",
        ));
    }
    if is_in(method, WRITE_METHODS) && !has(s::WRITE) {
        return Some(ErrorShape::new(
            error_codes::INVALID_REQUEST,
            "missing scope: operator.write",
        ));
    }

    if is_in(method, APPROVAL_METHODS)
        || is_in(method, PAIRING_METHODS)
        || is_in(method, READ_METHODS)
        || is_in(method, WRITE_METHODS)
    {
        return None;
    }

    Some(ErrorShape::new(
        error_codes::INVALID_REQUEST,
        "missing scope: operator.admin",
    ))
}

// ── Method registry ──────────────────────────────────────────────────────────

pub struct MethodRegistry {
    handlers: HashMap<String, HandlerFn>,
}

impl Default for MethodRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MethodRegistry {
    pub fn new() -> Self {
        let mut reg = Self {
            handlers: HashMap::new(),
        };
        reg.register_defaults();
        reg
    }

    pub fn register(&mut self, method: impl Into<String>, handler: HandlerFn) {
        self.handlers.insert(method.into(), handler);
    }

    pub async fn dispatch(&self, ctx: MethodContext) -> ResponseFrame {
        let method = ctx.method.clone();
        let request_id = ctx.request_id.clone();
        let conn_id = ctx.client_conn_id.clone();

        if let Some(err) = authorize_method(&method, &ctx.client_role, &ctx.client_scopes) {
            warn!(method, conn_id = %conn_id, code = %err.code, "method auth denied");
            return ResponseFrame::err(&request_id, err);
        }

        let Some(handler) = self.handlers.get(&method) else {
            warn!(method, conn_id = %conn_id, "unknown method");
            return ResponseFrame::err(
                &request_id,
                ErrorShape::new(
                    error_codes::INVALID_REQUEST,
                    format!("unknown method: {method}"),
                ),
            );
        };

        debug!(method, request_id = %request_id, conn_id = %conn_id, "dispatching method");
        match handler(ctx).await {
            Ok(payload) => {
                debug!(method, request_id = %request_id, "method ok");
                ResponseFrame::ok(&request_id, payload)
            },
            Err(err) => {
                warn!(method, request_id = %request_id, code = %err.code, msg = %err.message, "method error");
                ResponseFrame::err(&request_id, err)
            },
        }
    }

    pub fn method_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.handlers.keys().cloned().collect();
        names.sort();
        names
    }

    fn register_defaults(&mut self) {
        self.register_gateway_methods();
        self.register_node_methods();
        self.register_pairing_methods();
        self.register_service_methods();
    }

    // ── Gateway-internal methods ─────────────────────────────────────────

    fn register_gateway_methods(&mut self) {
        // health
        self.register(
            "health",
            Box::new(|ctx| {
                Box::pin(async move {
                    let count = ctx.state.client_count().await;
                    Ok(serde_json::json!({
                        "status": "ok",
                        "version": ctx.state.version,
                        "connections": count,
                    }))
                })
            }),
        );

        // status
        self.register(
            "status",
            Box::new(|ctx| {
                Box::pin(async move {
                    let nodes = ctx.state.nodes.read().await;
                    Ok(serde_json::json!({
                        "version": ctx.state.version,
                        "hostname": ctx.state.hostname,
                        "connections": ctx.state.client_count().await,
                        "nodes": nodes.count(),
                        "hasMobileNode": nodes.has_mobile_node(),
                    }))
                })
            }),
        );

        // system-presence
        self.register(
            "system-presence",
            Box::new(|ctx| {
                Box::pin(async move {
                    let clients = ctx.state.clients.read().await;
                    let nodes = ctx.state.nodes.read().await;

                    let client_list: Vec<_> = clients
                        .values()
                        .map(|c| {
                            serde_json::json!({
                                "connId": c.conn_id,
                                "clientId": c.connect_params.client.id,
                                "role": c.role(),
                                "platform": c.connect_params.client.platform,
                                "connectedAt": c.connected_at.elapsed().as_secs(),
                                "lastActivity": c.last_activity.elapsed().as_secs(),
                            })
                        })
                        .collect();

                    let node_list: Vec<_> = nodes
                        .list()
                        .iter()
                        .map(|n| {
                            serde_json::json!({
                                "nodeId": n.node_id,
                                "displayName": n.display_name,
                                "platform": n.platform,
                                "version": n.version,
                                "capabilities": n.capabilities,
                                "commands": n.commands,
                                "connectedAt": n.connected_at.elapsed().as_secs(),
                            })
                        })
                        .collect();

                    Ok(serde_json::json!({
                        "clients": client_list,
                        "nodes": node_list,
                    }))
                })
            }),
        );

        // system-event: broadcast an event to all operator clients
        self.register(
            "system-event",
            Box::new(|ctx| {
                Box::pin(async move {
                    let event = ctx
                        .params
                        .get("event")
                        .and_then(|v| v.as_str())
                        .unwrap_or("system");
                    let payload = ctx
                        .params
                        .get("payload")
                        .cloned()
                        .unwrap_or(serde_json::json!({}));
                    broadcast(&ctx.state, event, payload, BroadcastOpts::default()).await;
                    Ok(serde_json::json!({}))
                })
            }),
        );

        // last-heartbeat
        self.register(
            "last-heartbeat",
            Box::new(|ctx| {
                Box::pin(async move {
                    let clients = ctx.state.clients.read().await;
                    if let Some(client) = clients.get(&ctx.client_conn_id) {
                        Ok(serde_json::json!({
                            "lastActivitySecs": client.last_activity.elapsed().as_secs(),
                        }))
                    } else {
                        Ok(serde_json::json!({ "lastActivitySecs": 0 }))
                    }
                })
            }),
        );

        // set-heartbeats (touch activity for the caller)
        self.register(
            "set-heartbeats",
            Box::new(|ctx| {
                Box::pin(async move {
                    if let Some(client) =
                        ctx.state.clients.write().await.get_mut(&ctx.client_conn_id)
                    {
                        client.touch();
                    }
                    Ok(serde_json::json!({}))
                })
            }),
        );
    }

    // ── Node methods ─────────────────────────────────────────────────────

    fn register_node_methods(&mut self) {
        // node.list
        self.register(
            "node.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    let nodes = ctx.state.nodes.read().await;
                    let list: Vec<_> = nodes
                        .list()
                        .iter()
                        .map(|n| {
                            serde_json::json!({
                                "nodeId": n.node_id,
                                "displayName": n.display_name,
                                "platform": n.platform,
                                "version": n.version,
                                "capabilities": n.capabilities,
                                "commands": n.commands,
                                "remoteIp": n.remote_ip,
                            })
                        })
                        .collect();
                    Ok(serde_json::json!(list))
                })
            }),
        );

        // node.describe
        self.register(
            "node.describe",
            Box::new(|ctx| {
                Box::pin(async move {
                    let node_id = ctx
                        .params
                        .get("nodeId")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing nodeId")
                        })?;
                    let nodes = ctx.state.nodes.read().await;
                    let node = nodes.get(node_id).ok_or_else(|| {
                        ErrorShape::new(error_codes::UNAVAILABLE, "node not found")
                    })?;
                    Ok(serde_json::json!({
                        "nodeId": node.node_id,
                        "displayName": node.display_name,
                        "platform": node.platform,
                        "version": node.version,
                        "capabilities": node.capabilities,
                        "commands": node.commands,
                        "permissions": node.permissions,
                        "pathEnv": node.path_env,
                        "remoteIp": node.remote_ip,
                        "connectedAt": node.connected_at.elapsed().as_secs(),
                    }))
                })
            }),
        );

        // node.rename
        self.register(
            "node.rename",
            Box::new(|ctx| {
                Box::pin(async move {
                    let node_id = ctx
                        .params
                        .get("nodeId")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing nodeId")
                        })?;
                    let name = ctx
                        .params
                        .get("displayName")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing displayName")
                        })?;
                    let mut nodes = ctx.state.nodes.write().await;
                    nodes
                        .rename(node_id, name)
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))?;
                    Ok(serde_json::json!({}))
                })
            }),
        );

        // node.invoke: forward an RPC request to a connected node
        self.register(
            "node.invoke",
            Box::new(|ctx| {
                Box::pin(async move {
                    let node_id = ctx
                        .params
                        .get("nodeId")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing nodeId")
                        })?
                        .to_string();
                    let command = ctx
                        .params
                        .get("command")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing command")
                        })?
                        .to_string();
                    let args = ctx
                        .params
                        .get("args")
                        .cloned()
                        .unwrap_or(serde_json::json!({}));

                    // Find the node's conn_id and send the invoke request.
                    let invoke_id = uuid::Uuid::new_v4().to_string();
                    let conn_id = {
                        let nodes = ctx.state.nodes.read().await;
                        let node = nodes.get(&node_id).ok_or_else(|| {
                            ErrorShape::new(error_codes::UNAVAILABLE, "node not connected")
                        })?;
                        node.conn_id.clone()
                    };

                    // Send invoke event to the node.
                    let invoke_event = moltis_protocol::EventFrame::new(
                        "node.invoke.request",
                        serde_json::json!({
                            "invokeId": invoke_id,
                            "command": command,
                            "args": args,
                        }),
                        ctx.state.next_seq(),
                    );
                    let event_json = serde_json::to_string(&invoke_event).map_err(|e| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, e.to_string())
                    })?;

                    let clients = ctx.state.clients.read().await;
                    let node_client = clients.get(&conn_id).ok_or_else(|| {
                        ErrorShape::new(error_codes::UNAVAILABLE, "node connection lost")
                    })?;
                    if !node_client.send(&event_json) {
                        return Err(ErrorShape::new(
                            error_codes::UNAVAILABLE,
                            "node send failed",
                        ));
                    }
                    drop(clients);

                    // Set up a oneshot for the result with a timeout.
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    {
                        let mut invokes = ctx.state.pending_invokes.write().await;
                        invokes.insert(invoke_id.clone(), crate::state::PendingInvoke {
                            request_id: ctx.request_id.clone(),
                            sender: tx,
                            created_at: std::time::Instant::now(),
                        });
                    }

                    // Wait for result with 30s timeout.
                    match tokio::time::timeout(Duration::from_secs(30), rx).await {
                        Ok(Ok(result)) => Ok(result),
                        Ok(Err(_)) => Err(ErrorShape::new(
                            error_codes::UNAVAILABLE,
                            "invoke cancelled",
                        )),
                        Err(_) => {
                            ctx.state.pending_invokes.write().await.remove(&invoke_id);
                            Err(ErrorShape::new(
                                error_codes::AGENT_TIMEOUT,
                                "node invoke timeout",
                            ))
                        },
                    }
                })
            }),
        );

        // node.invoke.result: node returns the result of an invoke
        self.register(
            "node.invoke.result",
            Box::new(|ctx| {
                Box::pin(async move {
                    let invoke_id = ctx
                        .params
                        .get("invokeId")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing invokeId")
                        })?;
                    let result = ctx
                        .params
                        .get("result")
                        .cloned()
                        .unwrap_or(serde_json::json!(null));

                    let pending = ctx.state.pending_invokes.write().await.remove(invoke_id);
                    if let Some(invoke) = pending {
                        let _ = invoke.sender.send(result);
                        Ok(serde_json::json!({}))
                    } else {
                        Err(ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            "no pending invoke for this id",
                        ))
                    }
                })
            }),
        );

        // node.event: node broadcasts an event to operator clients
        self.register(
            "node.event",
            Box::new(|ctx| {
                Box::pin(async move {
                    let event = ctx
                        .params
                        .get("event")
                        .and_then(|v| v.as_str())
                        .unwrap_or("node.event");
                    let payload = ctx
                        .params
                        .get("payload")
                        .cloned()
                        .unwrap_or(serde_json::json!({}));
                    broadcast(&ctx.state, event, payload, BroadcastOpts::default()).await;
                    Ok(serde_json::json!({}))
                })
            }),
        );

        // logs.tail
        self.register(
            "logs.tail",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .logs
                        .tail(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // logs.list
        self.register(
            "logs.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .logs
                        .list(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // logs.status
        self.register(
            "logs.status",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .logs
                        .status()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // logs.ack
        self.register(
            "logs.ack",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .logs
                        .ack()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
    }

    // ── Pairing methods ──────────────────────────────────────────────────

    fn register_pairing_methods(&mut self) {
        // node.pair.request
        self.register(
            "node.pair.request",
            Box::new(|ctx| {
                Box::pin(async move {
                    let device_id = ctx
                        .params
                        .get("deviceId")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing deviceId")
                        })?;
                    let display_name = ctx.params.get("displayName").and_then(|v| v.as_str());
                    let platform = ctx
                        .params
                        .get("platform")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let public_key = ctx.params.get("publicKey").and_then(|v| v.as_str());

                    let req = ctx.state.pairing.write().await.request_pair(
                        device_id,
                        display_name,
                        platform,
                        public_key,
                    );

                    // Broadcast pair request to operators with pairing scope.
                    broadcast(
                        &ctx.state,
                        "node.pair.requested",
                        serde_json::json!({
                            "id": req.id,
                            "deviceId": req.device_id,
                            "displayName": req.display_name,
                            "platform": req.platform,
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    Ok(serde_json::json!({
                        "id": req.id,
                        "nonce": req.nonce,
                    }))
                })
            }),
        );

        // node.pair.list
        self.register(
            "node.pair.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    let pairing = ctx.state.pairing.read().await;
                    let list: Vec<_> = pairing
                        .list_pending()
                        .iter()
                        .map(|r| {
                            serde_json::json!({
                                "id": r.id,
                                "deviceId": r.device_id,
                                "displayName": r.display_name,
                                "platform": r.platform,
                            })
                        })
                        .collect();
                    Ok(serde_json::json!(list))
                })
            }),
        );

        // node.pair.approve
        self.register(
            "node.pair.approve",
            Box::new(|ctx| {
                Box::pin(async move {
                    let pair_id =
                        ctx.params
                            .get("id")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                ErrorShape::new(error_codes::INVALID_REQUEST, "missing id")
                            })?;
                    let token = ctx
                        .state
                        .pairing
                        .write()
                        .await
                        .approve(pair_id)
                        .map_err(|e| ErrorShape::new(error_codes::INVALID_REQUEST, e))?;

                    broadcast(
                        &ctx.state,
                        "node.pair.resolved",
                        serde_json::json!({
                            "id": pair_id, "status": "approved",
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    Ok(serde_json::json!({
                        "deviceToken": token.token,
                        "scopes": token.scopes,
                    }))
                })
            }),
        );

        // node.pair.reject
        self.register(
            "node.pair.reject",
            Box::new(|ctx| {
                Box::pin(async move {
                    let pair_id =
                        ctx.params
                            .get("id")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                ErrorShape::new(error_codes::INVALID_REQUEST, "missing id")
                            })?;
                    ctx.state
                        .pairing
                        .write()
                        .await
                        .reject(pair_id)
                        .map_err(|e| ErrorShape::new(error_codes::INVALID_REQUEST, e))?;

                    broadcast(
                        &ctx.state,
                        "node.pair.resolved",
                        serde_json::json!({
                            "id": pair_id, "status": "rejected",
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    Ok(serde_json::json!({}))
                })
            }),
        );

        // node.pair.verify (placeholder — signature verification)
        self.register(
            "node.pair.verify",
            Box::new(|_ctx| Box::pin(async move { Ok(serde_json::json!({ "verified": true })) })),
        );

        // device.pair.list
        self.register(
            "device.pair.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    let pairing = ctx.state.pairing.read().await;
                    let list: Vec<_> = pairing
                        .list_devices()
                        .iter()
                        .map(|d| {
                            serde_json::json!({
                                "deviceId": d.device_id,
                                "scopes": d.scopes,
                                "issuedAtMs": d.issued_at_ms,
                            })
                        })
                        .collect();
                    Ok(serde_json::json!(list))
                })
            }),
        );

        // device.pair.approve (alias for node.pair.approve)
        self.register(
            "device.pair.approve",
            Box::new(|ctx| {
                Box::pin(async move {
                    let pair_id =
                        ctx.params
                            .get("id")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                ErrorShape::new(error_codes::INVALID_REQUEST, "missing id")
                            })?;
                    let token = ctx
                        .state
                        .pairing
                        .write()
                        .await
                        .approve(pair_id)
                        .map_err(|e| ErrorShape::new(error_codes::INVALID_REQUEST, e))?;

                    broadcast(
                        &ctx.state,
                        "device.pair.resolved",
                        serde_json::json!({
                            "id": pair_id, "status": "approved",
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    Ok(serde_json::json!({ "deviceToken": token.token, "scopes": token.scopes }))
                })
            }),
        );

        // device.pair.reject
        self.register(
            "device.pair.reject",
            Box::new(|ctx| {
                Box::pin(async move {
                    let pair_id =
                        ctx.params
                            .get("id")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                ErrorShape::new(error_codes::INVALID_REQUEST, "missing id")
                            })?;
                    ctx.state
                        .pairing
                        .write()
                        .await
                        .reject(pair_id)
                        .map_err(|e| ErrorShape::new(error_codes::INVALID_REQUEST, e))?;

                    broadcast(
                        &ctx.state,
                        "device.pair.resolved",
                        serde_json::json!({
                            "id": pair_id, "status": "rejected",
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    Ok(serde_json::json!({}))
                })
            }),
        );

        // device.token.rotate
        self.register(
            "device.token.rotate",
            Box::new(|ctx| {
                Box::pin(async move {
                    let device_id = ctx
                        .params
                        .get("deviceId")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing deviceId")
                        })?;
                    let token = ctx
                        .state
                        .pairing
                        .write()
                        .await
                        .rotate_token(device_id)
                        .map_err(|e| ErrorShape::new(error_codes::INVALID_REQUEST, e))?;
                    Ok(serde_json::json!({ "deviceToken": token.token, "scopes": token.scopes }))
                })
            }),
        );

        // device.token.revoke
        self.register(
            "device.token.revoke",
            Box::new(|ctx| {
                Box::pin(async move {
                    let device_id = ctx
                        .params
                        .get("deviceId")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing deviceId")
                        })?;
                    ctx.state
                        .pairing
                        .write()
                        .await
                        .revoke_token(device_id)
                        .map_err(|e| ErrorShape::new(error_codes::INVALID_REQUEST, e))?;
                    Ok(serde_json::json!({}))
                })
            }),
        );
    }

    // ── Service-delegated methods ────────────────────────────────────────

    fn register_service_methods(&mut self) {
        // Agent
        self.register(
            "agent",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .agent
                        .run(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "agent.wait",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .agent
                        .run_wait(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "agent.identity.get",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .onboarding
                        .identity_get()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "agent.identity.update",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .onboarding
                        .identity_update(ctx.params)
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "agent.identity.update_soul",
            Box::new(|ctx| {
                Box::pin(async move {
                    let soul = ctx
                        .params
                        .get("soul")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    ctx.state
                        .services
                        .onboarding
                        .identity_update_soul(soul)
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "agents.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .agent
                        .list()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // Sessions
        self.register(
            "sessions.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .session
                        .list()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "sessions.preview",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .session
                        .preview(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "sessions.search",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .session
                        .search(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "sessions.resolve",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .session
                        .resolve(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "sessions.patch",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .session
                        .patch(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "sessions.reset",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .session
                        .reset(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "sessions.delete",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .session
                        .delete(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "sessions.compact",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .session
                        .compact(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        self.register(
            "sessions.fork",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .session
                        .fork(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "sessions.branches",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .session
                        .branches(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // Channels
        self.register(
            "channels.status",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .channel
                        .status()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        // channels.list is an alias for channels.status (used by the UI)
        self.register(
            "channels.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .channel
                        .status()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "channels.add",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .channel
                        .add(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "channels.remove",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .channel
                        .remove(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "channels.update",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .channel
                        .update(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "channels.logout",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .channel
                        .logout(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "channels.senders.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .channel
                        .senders_list(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "channels.senders.approve",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .channel
                        .sender_approve(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "channels.senders.deny",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .channel
                        .sender_deny(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "send",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .channel
                        .send(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // Config
        self.register(
            "config.get",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .config
                        .get(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "config.set",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .config
                        .set(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "config.apply",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .config
                        .apply(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "config.patch",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .config
                        .patch(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "config.schema",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .config
                        .schema()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // Cron
        self.register(
            "cron.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .cron
                        .list()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "cron.status",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .cron
                        .status()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "cron.add",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .cron
                        .add(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "cron.update",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .cron
                        .update(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "cron.remove",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .cron
                        .remove(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "cron.run",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .cron
                        .run(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "cron.runs",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .cron
                        .runs(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // Heartbeat
        self.register(
            "heartbeat.status",
            Box::new(|ctx| {
                Box::pin(async move {
                    let config = ctx.state.heartbeat_config.read().await.clone();
                    let heartbeat_path = moltis_config::heartbeat_path();
                    let heartbeat_file_exists = heartbeat_path.exists();
                    let heartbeat_md = moltis_config::load_heartbeat_md();
                    let (_, prompt_source) = moltis_cron::heartbeat::resolve_heartbeat_prompt(
                        config.prompt.as_deref(),
                        heartbeat_md.as_deref(),
                    );
                    let has_prompt_override = config
                        .prompt
                        .as_deref()
                        .is_some_and(|p| !p.trim().is_empty());
                    let heartbeat_file_effectively_empty =
                        heartbeat_file_exists && heartbeat_md.is_none();
                    let skip_llm_when_empty =
                        heartbeat_file_effectively_empty && !has_prompt_override;
                    // Find the heartbeat job to get its state.
                    let jobs_val = ctx
                        .state
                        .services
                        .cron
                        .list()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))?;
                    let jobs: Vec<moltis_cron::types::CronJob> =
                        serde_json::from_value(jobs_val).unwrap_or_default();
                    let hb_job = jobs.iter().find(|j| j.name == "__heartbeat__");
                    Ok(serde_json::json!({
                        "config": config,
                        "job": hb_job,
                        "promptSource": prompt_source.as_str(),
                        "heartbeatFileExists": heartbeat_file_exists,
                        "heartbeatFileEffectivelyEmpty": heartbeat_file_effectively_empty,
                        "skipLlmWhenEmpty": skip_llm_when_empty,
                    }))
                })
            }),
        );
        self.register(
            "heartbeat.update",
            Box::new(|ctx| {
                Box::pin(async move {
                    let patch: moltis_config::schema::HeartbeatConfig =
                        serde_json::from_value(ctx.params.clone()).map_err(|e| {
                            ErrorShape::new(
                                error_codes::INVALID_REQUEST,
                                format!("invalid heartbeat config: {e}"),
                            )
                        })?;
                    *ctx.state.heartbeat_config.write().await = patch.clone();

                    // Persist to moltis.toml so the config survives restarts.
                    if let Err(e) = moltis_config::update_config(|cfg| {
                        cfg.heartbeat = patch.clone();
                    }) {
                        tracing::warn!(error = %e, "failed to persist heartbeat config");
                    }

                    // Update the heartbeat cron job in-place.
                    let jobs_val = ctx
                        .state
                        .services
                        .cron
                        .list()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))?;
                    let jobs: Vec<moltis_cron::types::CronJob> =
                        serde_json::from_value(jobs_val).unwrap_or_default();
                    if let Some(hb_job) = jobs.iter().find(|j| j.name == "__heartbeat__") {
                        let interval_ms = moltis_cron::heartbeat::parse_interval_ms(&patch.every)
                            .unwrap_or(moltis_cron::heartbeat::DEFAULT_INTERVAL_MS);
                        let heartbeat_md = moltis_config::load_heartbeat_md();
                        let (prompt, prompt_source) =
                            moltis_cron::heartbeat::resolve_heartbeat_prompt(
                                patch.prompt.as_deref(),
                                heartbeat_md.as_deref(),
                            );
                        if prompt_source
                            == moltis_cron::heartbeat::HeartbeatPromptSource::HeartbeatMd
                        {
                            tracing::info!("loaded heartbeat prompt from HEARTBEAT.md");
                        }
                        if patch.prompt.as_deref().is_some_and(|p| !p.trim().is_empty())
                            && heartbeat_md.as_deref().is_some_and(|p| !p.trim().is_empty())
                            && prompt_source
                                == moltis_cron::heartbeat::HeartbeatPromptSource::Config
                        {
                            tracing::warn!(
                                "heartbeat prompt source conflict: config heartbeat.prompt overrides HEARTBEAT.md"
                            );
                        }
                        let job_patch = moltis_cron::types::CronJobPatch {
                            schedule: Some(moltis_cron::types::CronSchedule::Every {
                                every_ms: interval_ms,
                                anchor_ms: None,
                            }),
                            payload: Some(moltis_cron::types::CronPayload::AgentTurn {
                                message: prompt,
                                model: patch.model.clone(),
                                timeout_secs: None,
                                deliver: false,
                                channel: None,
                                to: None,
                            }),
                            enabled: Some(patch.enabled),
                            sandbox: Some(moltis_cron::types::CronSandboxConfig {
                                enabled: patch.sandbox_enabled,
                                image: patch.sandbox_image.clone(),
                            }),
                            ..Default::default()
                        };
                        ctx.state
                            .services
                            .cron
                            .update(serde_json::json!({
                                "id": hb_job.id,
                                "patch": job_patch,
                            }))
                            .await
                            .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))?;
                    }
                    Ok(serde_json::json!({ "updated": true }))
                })
            }),
        );
        self.register(
            "heartbeat.run",
            Box::new(|ctx| {
                Box::pin(async move {
                    let jobs_val = ctx
                        .state
                        .services
                        .cron
                        .list()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))?;
                    let jobs: Vec<moltis_cron::types::CronJob> =
                        serde_json::from_value(jobs_val).unwrap_or_default();
                    let hb_job =
                        jobs.iter()
                            .find(|j| j.name == "__heartbeat__")
                            .ok_or_else(|| {
                                ErrorShape::new(
                                    error_codes::INVALID_REQUEST,
                                    "heartbeat job not found",
                                )
                            })?;
                    ctx.state
                        .services
                        .cron
                        .run(serde_json::json!({
                            "id": hb_job.id,
                            "force": true,
                        }))
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))?;
                    Ok(serde_json::json!({ "triggered": true }))
                })
            }),
        );
        self.register(
            "heartbeat.runs",
            Box::new(|ctx| {
                Box::pin(async move {
                    let jobs_val = ctx
                        .state
                        .services
                        .cron
                        .list()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))?;
                    let jobs: Vec<moltis_cron::types::CronJob> =
                        serde_json::from_value(jobs_val).unwrap_or_default();
                    let hb_job =
                        jobs.iter()
                            .find(|j| j.name == "__heartbeat__")
                            .ok_or_else(|| {
                                ErrorShape::new(
                                    error_codes::INVALID_REQUEST,
                                    "heartbeat job not found",
                                )
                            })?;
                    let limit = ctx
                        .params
                        .get("limit")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(20);
                    ctx.state
                        .services
                        .cron
                        .runs(serde_json::json!({
                            "id": hb_job.id,
                            "limit": limit,
                        }))
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // Chat (uses chat_override if set, otherwise falls back to services.chat)
        // Inject _conn_id and _accept_language so the chat service can resolve
        // the active session and forward the user's locale to web tools.
        self.register(
            "chat.send",
            Box::new(|ctx| {
                Box::pin(async move {
                    let mut params = ctx.params.clone();
                    params["_conn_id"] = serde_json::json!(ctx.client_conn_id);
                    // Forward client Accept-Language to web tools.
                    let accept_language = {
                        let clients = ctx.state.clients.read().await;
                        clients
                            .get(&ctx.client_conn_id)
                            .and_then(|c| c.accept_language.clone())
                    };
                    if let Some(lang) = accept_language {
                        params["_accept_language"] = serde_json::json!(lang);
                    }
                    ctx.state
                        .chat()
                        .await
                        .send(params)
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "chat.abort",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .chat()
                        .await
                        .abort(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "chat.history",
            Box::new(|ctx| {
                Box::pin(async move {
                    let mut params = ctx.params.clone();
                    params["_conn_id"] = serde_json::json!(ctx.client_conn_id);
                    ctx.state
                        .chat()
                        .await
                        .history(params)
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "chat.inject",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .chat()
                        .await
                        .inject(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "chat.clear",
            Box::new(|ctx| {
                Box::pin(async move {
                    let mut params = ctx.params.clone();
                    params["_conn_id"] = serde_json::json!(ctx.client_conn_id);
                    ctx.state
                        .chat()
                        .await
                        .clear(params)
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "chat.compact",
            Box::new(|ctx| {
                Box::pin(async move {
                    let mut params = ctx.params.clone();
                    params["_conn_id"] = serde_json::json!(ctx.client_conn_id);
                    ctx.state
                        .chat()
                        .await
                        .compact(params)
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        self.register(
            "chat.context",
            Box::new(|ctx| {
                Box::pin(async move {
                    let mut params = ctx.params.clone();
                    params["_conn_id"] = serde_json::json!(ctx.client_conn_id);
                    ctx.state
                        .chat()
                        .await
                        .context(params)
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // Session switching
        self.register(
            "sessions.switch",
            Box::new(|ctx| {
                Box::pin(async move {
                    let key = ctx
                        .params
                        .get("key")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing 'key' parameter")
                        })?;

                    // Store the active session for this connection.
                    ctx.state
                        .active_sessions
                        .write()
                        .await
                        .insert(ctx.client_conn_id.clone(), key.to_string());

                    // Store the active project for this connection, if provided.
                    if let Some(project_id) = ctx.params.get("project_id").and_then(|v| v.as_str())
                    {
                        if project_id.is_empty() {
                            ctx.state
                                .active_projects
                                .write()
                                .await
                                .remove(&ctx.client_conn_id);
                        } else {
                            ctx.state
                                .active_projects
                                .write()
                                .await
                                .insert(ctx.client_conn_id.clone(), project_id.to_string());
                        }
                    }

                    // Resolve first (auto-creates session if needed), then
                    // persist project_id so the entry exists when we patch.
                    let result = ctx
                        .state
                        .services
                        .session
                        .resolve(serde_json::json!({ "key": key }))
                        .await
                        .map_err(|e| {
                            tracing::error!("session resolve failed: {e}");
                            ErrorShape::new(
                                error_codes::UNAVAILABLE,
                                format!("session resolve failed: {e}"),
                            )
                        })?;

                    if let Some(pid) = ctx.params.get("project_id").and_then(|v| v.as_str()) {
                        let _ = ctx
                            .state
                            .services
                            .session
                            .patch(serde_json::json!({ "key": key, "project_id": pid }))
                            .await;

                        // Auto-create worktree if project has auto_worktree enabled.
                        if let Ok(proj_val) = ctx
                            .state
                            .services
                            .project
                            .get(serde_json::json!({"id": pid}))
                            .await
                            && proj_val
                                .get("auto_worktree")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                            && let Some(dir) = proj_val.get("directory").and_then(|v| v.as_str())
                        {
                            let project_dir = std::path::Path::new(dir);
                            let create_result =
                                match moltis_projects::WorktreeManager::resolve_base_branch(
                                    project_dir,
                                )
                                .await
                                {
                                    Ok(base) => {
                                        moltis_projects::WorktreeManager::create_from_base(
                                            project_dir,
                                            key,
                                            &base,
                                        )
                                        .await
                                    },
                                    Err(_) => {
                                        moltis_projects::WorktreeManager::create(project_dir, key)
                                            .await
                                    },
                                };
                            match create_result {
                                Ok(wt_dir) => {
                                    let prefix = proj_val
                                        .get("branch_prefix")
                                        .and_then(|v| v.as_str())
                                        .filter(|s| !s.is_empty())
                                        .unwrap_or("moltis");
                                    let branch = format!("{prefix}/{key}");
                                    let _ = ctx
                                        .state
                                        .services
                                        .session
                                        .patch(serde_json::json!({
                                            "key": key,
                                            "worktree_branch": branch,
                                        }))
                                        .await;

                                    if let Err(e) = moltis_projects::worktree::copy_project_config(
                                        project_dir,
                                        &wt_dir,
                                    ) {
                                        tracing::warn!("failed to copy project config: {e}");
                                    }

                                    if let Some(cmd) = proj_val
                                        .get("setup_command")
                                        .and_then(|v| v.as_str())
                                        .filter(|s| !s.is_empty())
                                        && let Err(e) = moltis_projects::WorktreeManager::run_setup(
                                            &wt_dir,
                                            cmd,
                                            project_dir,
                                            key,
                                        )
                                        .await
                                    {
                                        tracing::warn!("worktree setup failed: {e}");
                                    }
                                },
                                Err(e) => {
                                    tracing::warn!("auto-create worktree failed: {e}");
                                },
                            }
                        }
                    }

                    Ok(result)
                })
            }),
        );

        // TTS and STT (voice feature)
        #[cfg(feature = "voice")]
        {
            self.register(
                "tts.status",
                Box::new(|ctx| {
                    Box::pin(async move {
                        ctx.state
                            .services
                            .tts
                            .status()
                            .await
                            .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                    })
                }),
            );
            self.register(
                "tts.providers",
                Box::new(|ctx| {
                    Box::pin(async move {
                        ctx.state
                            .services
                            .tts
                            .providers()
                            .await
                            .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                    })
                }),
            );
            self.register(
                "tts.enable",
                Box::new(|ctx| {
                    Box::pin(async move {
                        ctx.state
                            .services
                            .tts
                            .enable(ctx.params.clone())
                            .await
                            .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                    })
                }),
            );
            self.register(
                "tts.disable",
                Box::new(|ctx| {
                    Box::pin(async move {
                        ctx.state
                            .services
                            .tts
                            .disable()
                            .await
                            .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                    })
                }),
            );
            self.register(
                "tts.convert",
                Box::new(|ctx| {
                    Box::pin(async move {
                        ctx.state
                            .services
                            .tts
                            .convert(ctx.params.clone())
                            .await
                            .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                    })
                }),
            );
            self.register(
                "tts.setProvider",
                Box::new(|ctx| {
                    Box::pin(async move {
                        ctx.state
                            .services
                            .tts
                            .set_provider(ctx.params.clone())
                            .await
                            .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                    })
                }),
            );
            self.register(
                "stt.status",
                Box::new(|ctx| {
                    Box::pin(async move {
                        ctx.state
                            .services
                            .stt
                            .status()
                            .await
                            .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                    })
                }),
            );
            self.register(
                "stt.providers",
                Box::new(|ctx| {
                    Box::pin(async move {
                        ctx.state
                            .services
                            .stt
                            .providers()
                            .await
                            .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                    })
                }),
            );
            self.register(
                "stt.transcribe",
                Box::new(|ctx| {
                    Box::pin(async move {
                        ctx.state
                            .services
                            .stt
                            .transcribe(ctx.params.clone())
                            .await
                            .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                    })
                }),
            );
            self.register(
                "stt.setProvider",
                Box::new(|ctx| {
                    Box::pin(async move {
                        ctx.state
                            .services
                            .stt
                            .set_provider(ctx.params.clone())
                            .await
                            .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                    })
                }),
            );
        }

        // Skills
        self.register(
            "skills.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .skills
                        .list()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "skills.status",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .skills
                        .status()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "skills.bins",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .skills
                        .bins()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "skills.install",
            Box::new(|ctx| {
                Box::pin(async move {
                    let source = ctx
                        .params
                        .get("source")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let op_id = ctx
                        .params
                        .get("op_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(ctx.request_id.as_str())
                        .to_string();

                    broadcast(
                        &ctx.state,
                        "skills.install.progress",
                        serde_json::json!({
                            "phase": "start",
                            "source": source,
                            "op_id": op_id,
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    match ctx.state.services.skills.install(ctx.params.clone()).await {
                        Ok(payload) => {
                            broadcast(
                                &ctx.state,
                                "skills.install.progress",
                                serde_json::json!({
                                    "phase": "done",
                                    "source": source,
                                    "op_id": op_id,
                                }),
                                BroadcastOpts::default(),
                            )
                            .await;
                            Ok(payload)
                        },
                        Err(e) => {
                            broadcast(
                                &ctx.state,
                                "skills.install.progress",
                                serde_json::json!({
                                    "phase": "error",
                                    "source": source,
                                    "op_id": op_id,
                                    "error": e,
                                }),
                                BroadcastOpts::default(),
                            )
                            .await;
                            Err(ErrorShape::new(error_codes::UNAVAILABLE, e))
                        },
                    }
                })
            }),
        );
        self.register(
            "skills.remove",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .skills
                        .remove(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "skills.update",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .skills
                        .update(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "skills.repos.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .skills
                        .repos_list()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "skills.repos.remove",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .skills
                        .repos_remove(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "skills.emergency_disable",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .skills
                        .emergency_disable()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "skills.skill.trust",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .skills
                        .skill_trust(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "skills.skill.enable",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .skills
                        .skill_enable(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "skills.skill.disable",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .skills
                        .skill_disable(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "skills.skill.detail",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .skills
                        .skill_detail(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "skills.install_dep",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .skills
                        .install_dep(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // MCP
        self.register(
            "mcp.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .mcp
                        .list()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "mcp.add",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .mcp
                        .add(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "mcp.remove",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .mcp
                        .remove(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "mcp.enable",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .mcp
                        .enable(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "mcp.disable",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .mcp
                        .disable(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "mcp.status",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .mcp
                        .status(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "mcp.tools",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .mcp
                        .tools(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "mcp.restart",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .mcp
                        .restart(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "mcp.update",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .mcp
                        .update(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // Browser
        self.register(
            "browser.request",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .browser
                        .request(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // Usage
        self.register(
            "usage.status",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .usage
                        .status()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "usage.cost",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .usage
                        .cost(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // Exec approvals
        self.register(
            "exec.approvals.get",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .exec_approval
                        .get()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "exec.approvals.set",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .exec_approval
                        .set(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "exec.approvals.node.get",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .exec_approval
                        .node_get(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "exec.approvals.node.set",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .exec_approval
                        .node_set(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "exec.approval.request",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .exec_approval
                        .request(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "exec.approval.resolve",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .exec_approval
                        .resolve(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // Models
        self.register(
            "models.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .model
                        .list()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "models.list_all",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .model
                        .list_all()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "models.disable",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .model
                        .disable(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "models.enable",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .model
                        .enable(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "models.detect_supported",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .model
                        .detect_supported(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // Provider setup
        self.register(
            "providers.available",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .provider_setup
                        .available()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "providers.save_key",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .provider_setup
                        .save_key(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "providers.oauth.start",
            Box::new(|ctx| {
                Box::pin(async move {
                    let provider_name = ctx
                        .params
                        .get("provider")
                        .and_then(|v| v.as_str())
                        .map(ToOwned::to_owned);
                    let result = ctx
                        .state
                        .services
                        .provider_setup
                        .oauth_start(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))?;

                    // If oauth.start short-circuited because valid tokens already
                    // existed, trigger a provider-scoped background probe now.
                    if result
                        .get("alreadyAuthenticated")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        let model_service = Arc::clone(&ctx.state.services.model);
                        tokio::spawn(async move {
                            let _ = model_service
                                .detect_supported(model_probe_params(provider_name.as_deref()))
                                .await;
                        });
                    }

                    Ok(result)
                })
            }),
        );
        self.register(
            "providers.oauth.status",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .provider_setup
                        .oauth_status(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "providers.oauth.complete",
            Box::new(|ctx| {
                Box::pin(async move {
                    let result = ctx
                        .state
                        .services
                        .provider_setup
                        .oauth_complete(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))?;

                    let provider_name = result
                        .get("provider")
                        .and_then(|v| v.as_str())
                        .map(ToOwned::to_owned);

                    // Kick off background support probing after OAuth provider connect.
                    let model_service = Arc::clone(&ctx.state.services.model);
                    tokio::spawn(async move {
                        let _ = model_service
                            .detect_supported(model_probe_params(provider_name.as_deref()))
                            .await;
                    });

                    Ok(result)
                })
            }),
        );
        self.register(
            "providers.remove_key",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .provider_setup
                        .remove_key(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // Local LLM
        self.register(
            "providers.local.system_info",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .local_llm
                        .system_info()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "providers.local.models",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .local_llm
                        .models()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "providers.local.configure",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .local_llm
                        .configure(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "providers.local.status",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .local_llm
                        .status()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "providers.local.search_hf",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .local_llm
                        .search_hf(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "providers.local.configure_custom",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .local_llm
                        .configure_custom(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "providers.local.remove_model",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .local_llm
                        .remove_model(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // Voicewake
        self.register(
            "voicewake.get",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .voicewake
                        .get()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "voicewake.set",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .voicewake
                        .set(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "wake",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .voicewake
                        .wake(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "talk.mode",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .voicewake
                        .talk_mode(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // Update
        self.register(
            "update.run",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .update
                        .run(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // Onboarding / Wizard
        self.register(
            "wizard.start",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .onboarding
                        .wizard_start(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "wizard.next",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .onboarding
                        .wizard_next(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "wizard.cancel",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .onboarding
                        .wizard_cancel()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "wizard.status",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .onboarding
                        .wizard_status()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // Web login
        self.register(
            "web.login.start",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .web_login
                        .start(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "web.login.wait",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .web_login
                        .wait(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // ── Projects ────────────────────────────────────────────────────

        self.register(
            "projects.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .project
                        .list()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "projects.get",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .project
                        .get(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "projects.upsert",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .project
                        .upsert(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "projects.delete",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .project
                        .delete(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "projects.detect",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .project
                        .detect(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "projects.complete_path",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .project
                        .complete_path(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "projects.context",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .project
                        .context(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );

        // ── Voice Config ───────────────────────────────────────────────
        #[cfg(feature = "voice")]
        {
            self.register(
                "voice.config.get",
                Box::new(|_ctx| {
                    Box::pin(async move {
                        let config = moltis_config::discover_and_load();
                        Ok(serde_json::json!({
                            "tts": {
                                "enabled": config.voice.tts.enabled,
                                "provider": config.voice.tts.provider,
                                "elevenlabs_configured": config.voice.tts.elevenlabs.api_key.is_some(),
                                "openai_configured": config.voice.tts.openai.api_key.is_some(),
                            },
                            "stt": {
                                "enabled": config.voice.stt.enabled,
                                "provider": config.voice.stt.provider,
                                "whisper_configured": config.voice.stt.whisper.api_key.is_some(),
                                "groq_configured": config.voice.stt.groq.api_key.is_some(),
                                "deepgram_configured": config.voice.stt.deepgram.api_key.is_some(),
                                "google_configured": config.voice.stt.google.api_key.is_some(),
                                "elevenlabs_configured": config.voice.stt.elevenlabs.api_key.is_some(),
                                "whisper_cli_configured": config.voice.stt.whisper_cli.model_path.is_some(),
                                "sherpa_onnx_configured": config.voice.stt.sherpa_onnx.model_dir.is_some(),
                            },
                        }))
                    })
                }),
            );
            // Comprehensive provider listing with availability detection
            self.register(
                "voice.providers.all",
                Box::new(|_ctx| {
                    Box::pin(async move {
                        let config = moltis_config::discover_and_load();
                        let providers = detect_voice_providers(&config).await;
                        Ok(serde_json::json!(providers))
                    })
                }),
            );
            self.register(
                "voice.elevenlabs.catalog",
                Box::new(|_ctx| {
                    Box::pin(async move {
                        let config = moltis_config::discover_and_load();
                        Ok(fetch_elevenlabs_catalog(&config).await)
                    })
                }),
            );
            // Enable/disable a voice provider (updates config file)
            self.register(
                "voice.provider.toggle",
                Box::new(|ctx| {
                    Box::pin(async move {
                        let provider = ctx
                            .params
                            .get("provider")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                ErrorShape::new(error_codes::INVALID_REQUEST, "missing provider")
                            })?;
                        let enabled = ctx
                            .params
                            .get("enabled")
                            .and_then(|v| v.as_bool())
                            .ok_or_else(|| {
                                ErrorShape::new(error_codes::INVALID_REQUEST, "missing enabled")
                            })?;
                        let provider_type = ctx
                            .params
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("stt");

                        toggle_voice_provider(provider, enabled, provider_type).map_err(|e| {
                            ErrorShape::new(
                                error_codes::UNAVAILABLE,
                                format!("failed to toggle provider: {}", e),
                            )
                        })?;

                        // Broadcast change
                        broadcast(
                            &ctx.state,
                            "voice.config.changed",
                            serde_json::json!({ "provider": provider, "enabled": enabled }),
                            BroadcastOpts::default(),
                        )
                        .await;

                        Ok(serde_json::json!({ "ok": true, "provider": provider, "enabled": enabled }))
                    })
                }),
            );
            self.register(
                "voice.override.session.set",
                Box::new(|ctx| {
                    Box::pin(async move {
                        let session_key = ctx
                            .params
                            .get("sessionKey")
                            .or_else(|| ctx.params.get("session_key"))
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                ErrorShape::new(error_codes::INVALID_REQUEST, "missing sessionKey")
                            })?
                            .to_string();

                        let override_cfg = crate::state::TtsRuntimeOverride {
                            provider: ctx
                                .params
                                .get("provider")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                            voice_id: ctx
                                .params
                                .get("voiceId")
                                .or_else(|| ctx.params.get("voice_id"))
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                            model: ctx
                                .params
                                .get("model")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                        };

                        ctx.state
                            .tts_session_overrides
                            .write()
                            .await
                            .insert(session_key.clone(), override_cfg.clone());

                        Ok(serde_json::to_value(override_cfg).unwrap_or_else(
                            |_| serde_json::json!({ "ok": true, "sessionKey": session_key }),
                        ))
                    })
                }),
            );
            self.register(
                "voice.override.session.clear",
                Box::new(|ctx| {
                    Box::pin(async move {
                        let session_key = ctx
                            .params
                            .get("sessionKey")
                            .or_else(|| ctx.params.get("session_key"))
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                ErrorShape::new(error_codes::INVALID_REQUEST, "missing sessionKey")
                            })?
                            .to_string();

                        ctx.state
                            .tts_session_overrides
                            .write()
                            .await
                            .remove(&session_key);
                        Ok(serde_json::json!({ "ok": true, "sessionKey": session_key }))
                    })
                }),
            );
            self.register(
                "voice.override.channel.set",
                Box::new(|ctx| {
                    Box::pin(async move {
                        let channel_type = ctx
                            .params
                            .get("channelType")
                            .or_else(|| ctx.params.get("channel_type"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("telegram");
                        let account_id = ctx
                            .params
                            .get("accountId")
                            .or_else(|| ctx.params.get("account_id"))
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                ErrorShape::new(error_codes::INVALID_REQUEST, "missing accountId")
                            })?;

                        let key = format!("{}:{}", channel_type, account_id);
                        let override_cfg = crate::state::TtsRuntimeOverride {
                            provider: ctx
                                .params
                                .get("provider")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                            voice_id: ctx
                                .params
                                .get("voiceId")
                                .or_else(|| ctx.params.get("voice_id"))
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                            model: ctx
                                .params
                                .get("model")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                        };

                        ctx.state
                            .tts_channel_overrides
                            .write()
                            .await
                            .insert(key.clone(), override_cfg.clone());

                        Ok(serde_json::json!({ "ok": true, "key": key, "override": override_cfg }))
                    })
                }),
            );
            self.register(
                "voice.override.channel.clear",
                Box::new(|ctx| {
                    Box::pin(async move {
                        let channel_type = ctx
                            .params
                            .get("channelType")
                            .or_else(|| ctx.params.get("channel_type"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("telegram");
                        let account_id = ctx
                            .params
                            .get("accountId")
                            .or_else(|| ctx.params.get("account_id"))
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                ErrorShape::new(error_codes::INVALID_REQUEST, "missing accountId")
                            })?;

                        let key = format!("{}:{}", channel_type, account_id);
                        ctx.state.tts_channel_overrides.write().await.remove(&key);
                        Ok(serde_json::json!({ "ok": true, "key": key }))
                    })
                }),
            );
            self.register(
                "voice.config.save_key",
                Box::new(|ctx| {
                    Box::pin(async move {
                        use secrecy::Secret;

                        let provider = ctx
                            .params
                            .get("provider")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                ErrorShape::new(error_codes::INVALID_REQUEST, "missing provider")
                            })?;
                        let api_key = ctx
                            .params
                            .get("api_key")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                ErrorShape::new(error_codes::INVALID_REQUEST, "missing api_key")
                            })?;

                        moltis_config::update_config(|cfg| {
                            match provider {
                                // TTS providers
                                "elevenlabs" => {
                                    // ElevenLabs shares key between TTS and STT
                                    let key = Secret::new(api_key.to_string());
                                    cfg.voice.tts.elevenlabs.api_key = Some(key.clone());
                                    cfg.voice.stt.elevenlabs.api_key =
                                        Some(Secret::new(api_key.to_string()));
                                    // Auto-enable both TTS and STT with ElevenLabs
                                    cfg.voice.tts.provider = "elevenlabs".to_string();
                                    cfg.voice.tts.enabled = true;
                                    cfg.voice.stt.provider = VoiceSttProvider::ElevenLabs;
                                    cfg.voice.stt.enabled = true;
                                },
                                "openai" | "openai-tts" => {
                                    cfg.voice.tts.openai.api_key =
                                        Some(Secret::new(api_key.to_string()));
                                    cfg.voice.tts.provider = "openai".to_string();
                                    cfg.voice.tts.enabled = true;
                                },
                                "google-tts" => {
                                    // Google API key is shared - set both TTS and STT
                                    let key = Secret::new(api_key.to_string());
                                    cfg.voice.tts.google.api_key = Some(key.clone());
                                    cfg.voice.stt.google.api_key =
                                        Some(Secret::new(api_key.to_string()));
                                    // Auto-enable both TTS and STT with Google
                                    cfg.voice.tts.provider = "google".to_string();
                                    cfg.voice.tts.enabled = true;
                                    cfg.voice.stt.provider = VoiceSttProvider::Google;
                                    cfg.voice.stt.enabled = true;
                                },
                                // STT providers
                                "whisper" => {
                                    cfg.voice.stt.whisper.api_key =
                                        Some(Secret::new(api_key.to_string()));
                                    cfg.voice.stt.provider = VoiceSttProvider::Whisper;
                                    cfg.voice.stt.enabled = true;
                                },
                                "groq" => {
                                    cfg.voice.stt.groq.api_key =
                                        Some(Secret::new(api_key.to_string()));
                                    cfg.voice.stt.provider = VoiceSttProvider::Groq;
                                    cfg.voice.stt.enabled = true;
                                },
                                "deepgram" => {
                                    cfg.voice.stt.deepgram.api_key =
                                        Some(Secret::new(api_key.to_string()));
                                    cfg.voice.stt.provider = VoiceSttProvider::Deepgram;
                                    cfg.voice.stt.enabled = true;
                                },
                                "google" => {
                                    // Google STT key - also set TTS since they share the same key
                                    let key = Secret::new(api_key.to_string());
                                    cfg.voice.stt.google.api_key = Some(key.clone());
                                    cfg.voice.tts.google.api_key =
                                        Some(Secret::new(api_key.to_string()));
                                    cfg.voice.stt.provider = VoiceSttProvider::Google;
                                    cfg.voice.stt.enabled = true;
                                },
                                "mistral" => {
                                    cfg.voice.stt.mistral.api_key =
                                        Some(Secret::new(api_key.to_string()));
                                    cfg.voice.stt.provider = VoiceSttProvider::Mistral;
                                    cfg.voice.stt.enabled = true;
                                },
                                "elevenlabs-stt" => {
                                    // ElevenLabs shares key between TTS and STT
                                    let key = Secret::new(api_key.to_string());
                                    cfg.voice.stt.elevenlabs.api_key = Some(key.clone());
                                    cfg.voice.tts.elevenlabs.api_key =
                                        Some(Secret::new(api_key.to_string()));
                                    cfg.voice.stt.provider = VoiceSttProvider::ElevenLabs;
                                    cfg.voice.stt.enabled = true;
                                },
                                _ => {},
                            }

                            apply_voice_provider_settings(cfg, provider, &ctx.params);
                        })
                        .map_err(|e| {
                            ErrorShape::new(
                                error_codes::UNAVAILABLE,
                                format!("failed to save: {}", e),
                            )
                        })?;

                        // Broadcast voice config change event
                        broadcast(
                            &ctx.state,
                            "voice.config.changed",
                            serde_json::json!({ "provider": provider }),
                            BroadcastOpts::default(),
                        )
                        .await;

                        Ok(serde_json::json!({ "ok": true, "provider": provider }))
                    })
                }),
            );
            self.register(
                "voice.config.save_settings",
                Box::new(|ctx| {
                    Box::pin(async move {
                        let provider = ctx
                            .params
                            .get("provider")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                ErrorShape::new(error_codes::INVALID_REQUEST, "missing provider")
                            })?;

                        moltis_config::update_config(|cfg| {
                            apply_voice_provider_settings(cfg, provider, &ctx.params);
                        })
                        .map_err(|e| {
                            ErrorShape::new(
                                error_codes::UNAVAILABLE,
                                format!("failed to save settings: {}", e),
                            )
                        })?;

                        broadcast(
                            &ctx.state,
                            "voice.config.changed",
                            serde_json::json!({ "provider": provider, "settings": true }),
                            BroadcastOpts::default(),
                        )
                        .await;

                        Ok(serde_json::json!({ "ok": true, "provider": provider }))
                    })
                }),
            );
            self.register(
                "voice.config.remove_key",
                Box::new(|ctx| {
                    Box::pin(async move {
                        let provider = ctx
                            .params
                            .get("provider")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                ErrorShape::new(error_codes::INVALID_REQUEST, "missing provider")
                            })?;

                        moltis_config::update_config(|cfg| match provider {
                            // TTS providers
                            "elevenlabs" => {
                                cfg.voice.tts.elevenlabs.api_key = None;
                            },
                            "openai" => {
                                cfg.voice.tts.openai.api_key = None;
                            },
                            // STT providers
                            "whisper" => {
                                cfg.voice.stt.whisper.api_key = None;
                            },
                            "groq" => {
                                cfg.voice.stt.groq.api_key = None;
                            },
                            "deepgram" => {
                                cfg.voice.stt.deepgram.api_key = None;
                            },
                            "google" => {
                                cfg.voice.stt.google.api_key = None;
                            },
                            "mistral" => {
                                cfg.voice.stt.mistral.api_key = None;
                            },
                            "elevenlabs-stt" => {
                                cfg.voice.stt.elevenlabs.api_key = None;
                            },
                            _ => {},
                        })
                        .map_err(|e| {
                            ErrorShape::new(
                                error_codes::UNAVAILABLE,
                                format!("failed to save: {}", e),
                            )
                        })?;

                        // Broadcast voice config change event
                        broadcast(
                            &ctx.state,
                            "voice.config.changed",
                            serde_json::json!({ "provider": provider, "removed": true }),
                            BroadcastOpts::default(),
                        )
                        .await;

                        Ok(serde_json::json!({ "ok": true, "provider": provider }))
                    })
                }),
            );
            self.register(
                "voice.config.voxtral_requirements",
                Box::new(|_ctx| {
                    Box::pin(async move {
                        // Detect OS and architecture
                        let os = std::env::consts::OS;
                        let arch = std::env::consts::ARCH;

                        // Check Python version
                        let python_info = check_python_version().await;

                        // Check CUDA availability
                        let cuda_info = check_cuda_availability().await;

                        // Determine compatibility
                        let (compatible, reasons) =
                            check_voxtral_compatibility(os, arch, &python_info, &cuda_info);

                        Ok(serde_json::json!({
                            "os": os,
                            "arch": arch,
                            "python": python_info,
                            "cuda": cuda_info,
                            "compatible": compatible,
                            "reasons": reasons,
                        }))
                    })
                }),
            );
        }

        // ── Memory ─────────────────────────────────────────────────────

        self.register(
            "memory.status",
            Box::new(|ctx| {
                Box::pin(async move {
                    if let Some(ref mm) = ctx.state.memory_manager {
                        match mm.status().await {
                            Ok(status) => Ok(serde_json::json!({
                                "available": true,
                                "total_files": status.total_files,
                                "total_chunks": status.total_chunks,
                                "db_size": status.db_size_bytes,
                                "db_size_display": status.db_size_display(),
                                "embedding_model": status.embedding_model,
                                "has_embeddings": mm.has_embeddings(),
                            })),
                            Err(e) => Ok(serde_json::json!({
                                "available": false,
                                "error": e.to_string(),
                            })),
                        }
                    } else {
                        Ok(serde_json::json!({
                            "available": false,
                            "error": "Memory system not initialized",
                        }))
                    }
                })
            }),
        );

        self.register(
            "memory.config.get",
            Box::new(|_ctx| {
                Box::pin(async move {
                    // Read memory config from the config file
                    let config = moltis_config::discover_and_load();
                    let memory = &config.memory;
                    Ok(serde_json::json!({
                        "backend": memory.backend.as_deref().unwrap_or("builtin"),
                        "citations": memory.citations.as_deref().unwrap_or("auto"),
                        "llm_reranking": memory.llm_reranking,
                        "session_export": memory.session_export,
                        "qmd_feature_enabled": cfg!(feature = "qmd"),
                    }))
                })
            }),
        );

        self.register(
            "memory.config.update",
            Box::new(|ctx| {
                Box::pin(async move {
                    let backend = ctx
                        .params
                        .get("backend")
                        .and_then(|v| v.as_str())
                        .unwrap_or("builtin");
                    let citations = ctx
                        .params
                        .get("citations")
                        .and_then(|v| v.as_str())
                        .unwrap_or("auto");
                    let llm_reranking = ctx
                        .params
                        .get("llm_reranking")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let session_export = ctx
                        .params
                        .get("session_export")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    // Persist to moltis.toml so the config survives restarts.
                    let backend_str = backend.to_string();
                    let citations_str = citations.to_string();
                    if let Err(e) = moltis_config::update_config(|cfg| {
                        cfg.memory.backend = Some(backend_str.clone());
                        cfg.memory.citations = Some(citations_str.clone());
                        cfg.memory.llm_reranking = llm_reranking;
                        cfg.memory.session_export = session_export;
                    }) {
                        tracing::warn!(error = %e, "failed to persist memory config");
                    }

                    Ok(serde_json::json!({
                        "backend": backend,
                        "citations": citations,
                        "llm_reranking": llm_reranking,
                        "session_export": session_export,
                    }))
                })
            }),
        );

        // QMD status check
        self.register(
            "memory.qmd.status",
            Box::new(|_ctx| {
                Box::pin(async move {
                    #[cfg(feature = "qmd")]
                    {
                        use moltis_qmd::{QmdManager, QmdManagerConfig};

                        let config = moltis_config::discover_and_load();
                        let qmd_config = QmdManagerConfig {
                            command: config
                                .memory
                                .qmd
                                .command
                                .clone()
                                .unwrap_or_else(|| "qmd".into()),
                            collections: std::collections::HashMap::new(),
                            max_results: config.memory.qmd.max_results.unwrap_or(10),
                            timeout_ms: config.memory.qmd.timeout_ms.unwrap_or(30_000),
                            work_dir: moltis_config::data_dir(),
                        };

                        let manager = QmdManager::new(qmd_config);
                        let status = manager.status().await;

                        Ok(serde_json::json!({
                            "feature_enabled": true,
                            "available": status.available,
                            "version": status.version,
                            "error": status.error,
                        }))
                    }

                    #[cfg(not(feature = "qmd"))]
                    {
                        Ok(serde_json::json!({
                            "feature_enabled": false,
                            "available": false,
                            "error": "QMD feature not enabled. Rebuild with --features qmd",
                        }))
                    }
                })
            }),
        );

        // ── Hooks methods ────────────────────────────────────────────────

        // hooks.list — return discovered hooks with live stats.
        self.register(
            "hooks.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    let hooks = ctx.state.discovered_hooks.read().await;
                    let mut list = hooks.clone();

                    // Enrich with live stats from the registry.
                    if let Some(ref registry) = *ctx.state.hook_registry.read().await {
                        for hook in &mut list {
                            if let Some(stats) = registry.handler_stats(&hook.name) {
                                let calls =
                                    stats.call_count.load(std::sync::atomic::Ordering::Relaxed);
                                let failures = stats
                                    .failure_count
                                    .load(std::sync::atomic::Ordering::Relaxed);
                                let total_us = stats
                                    .total_latency_us
                                    .load(std::sync::atomic::Ordering::Relaxed);
                                hook.call_count = calls;
                                hook.failure_count = failures;
                                hook.avg_latency_ms =
                                    total_us.checked_div(calls).unwrap_or(0) / 1000;
                            }
                        }
                    }

                    Ok(serde_json::json!({ "hooks": list }))
                })
            }),
        );

        // hooks.enable — re-enable a previously disabled hook.
        self.register(
            "hooks.enable",
            Box::new(|ctx| {
                Box::pin(async move {
                    let name =
                        ctx.params
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                ErrorShape::new(error_codes::INVALID_REQUEST, "missing name")
                            })?;

                    ctx.state.disabled_hooks.write().await.remove(name);

                    // Persist disabled hooks list.
                    persist_disabled_hooks(&ctx.state).await;

                    // Rebuild hooks.
                    reload_hooks(&ctx.state).await;

                    Ok(serde_json::json!({ "ok": true }))
                })
            }),
        );

        // hooks.disable — disable a hook without removing its files.
        self.register(
            "hooks.disable",
            Box::new(|ctx| {
                Box::pin(async move {
                    let name =
                        ctx.params
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                ErrorShape::new(error_codes::INVALID_REQUEST, "missing name")
                            })?;

                    ctx.state
                        .disabled_hooks
                        .write()
                        .await
                        .insert(name.to_string());

                    // Persist disabled hooks list.
                    persist_disabled_hooks(&ctx.state).await;

                    // Rebuild hooks.
                    reload_hooks(&ctx.state).await;

                    Ok(serde_json::json!({ "ok": true }))
                })
            }),
        );

        // hooks.save — write HOOK.md content back to disk.
        self.register(
            "hooks.save",
            Box::new(|ctx| {
                Box::pin(async move {
                    let name =
                        ctx.params
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                ErrorShape::new(error_codes::INVALID_REQUEST, "missing name")
                            })?;
                    let content = ctx
                        .params
                        .get("content")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing content")
                        })?;

                    // Find the hook's source path.
                    let source_path = {
                        let hooks = ctx.state.discovered_hooks.read().await;
                        hooks
                            .iter()
                            .find(|h| h.name == name)
                            .map(|h| h.source_path.clone())
                    };

                    let source_path = source_path.ok_or_else(|| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, "hook not found")
                    })?;

                    // Write the content to HOOK.md.
                    let hook_md_path = std::path::PathBuf::from(&source_path).join("HOOK.md");
                    std::fs::write(&hook_md_path, content).map_err(|e| {
                        ErrorShape::new(
                            error_codes::UNAVAILABLE,
                            format!("failed to write HOOK.md: {e}"),
                        )
                    })?;

                    // Reload hooks to pick up the changes.
                    reload_hooks(&ctx.state).await;

                    Ok(serde_json::json!({ "ok": true }))
                })
            }),
        );

        // hooks.reload — re-run discovery and rebuild the registry.
        self.register(
            "hooks.reload",
            Box::new(|ctx| {
                Box::pin(async move {
                    reload_hooks(&ctx.state).await;
                    Ok(serde_json::json!({ "ok": true }))
                })
            }),
        );
    }
}

/// Check if Python 3.10+ is available.
async fn check_python_version() -> serde_json::Value {
    // Try python3 first, then python
    for cmd in &["python3", "python"] {
        if let Ok(output) = tokio::process::Command::new(cmd)
            .arg("--version")
            .output()
            .await
            && output.status.success()
        {
            let version_str = String::from_utf8_lossy(&output.stdout);
            // Parse "Python 3.11.0" format
            if let Some(version) = version_str.strip_prefix("Python ") {
                let version = version.trim();
                // Check if version is 3.10+
                let parts: Vec<&str> = version.split('.').collect();
                if parts.len() >= 2
                    && let (Ok(major), Ok(minor)) =
                        (parts[0].parse::<u32>(), parts[1].parse::<u32>())
                {
                    let sufficient = major > 3 || (major == 3 && minor >= 10);
                    return serde_json::json!({
                        "available": true,
                        "version": version,
                        "sufficient": sufficient,
                    });
                }
                return serde_json::json!({
                    "available": true,
                    "version": version,
                    "sufficient": false,
                });
            }
        }
    }
    serde_json::json!({
        "available": false,
        "version": null,
        "sufficient": false,
    })
}

/// Check CUDA availability via nvidia-smi.
async fn check_cuda_availability() -> serde_json::Value {
    // Check if nvidia-smi is available
    if let Ok(output) = tokio::process::Command::new("nvidia-smi")
        .arg("--query-gpu=name,memory.total")
        .arg("--format=csv,noheader,nounits")
        .output()
        .await
        && output.status.success()
    {
        let info = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = info.trim().lines().collect();
        if let Some(first_gpu) = lines.first() {
            let parts: Vec<&str> = first_gpu.split(", ").collect();
            if parts.len() >= 2 {
                let gpu_name = parts[0].trim();
                let memory_mb: u64 = parts[1].trim().parse().unwrap_or(0);
                // vLLM needs ~9.5GB, recommend 10GB minimum
                let sufficient = memory_mb >= 10000;
                return serde_json::json!({
                    "available": true,
                    "gpu_name": gpu_name,
                    "memory_mb": memory_mb,
                    "sufficient": sufficient,
                });
            }
        }
        return serde_json::json!({
            "available": true,
            "gpu_name": null,
            "memory_mb": null,
            "sufficient": false,
        });
    }
    serde_json::json!({
        "available": false,
        "gpu_name": null,
        "memory_mb": null,
        "sufficient": false,
    })
}

/// Check if the system meets Voxtral Local requirements.
fn check_voxtral_compatibility(
    os: &str,
    arch: &str,
    python: &serde_json::Value,
    cuda: &serde_json::Value,
) -> (bool, Vec<String>) {
    let mut reasons = Vec::new();

    // vLLM primarily supports Linux
    let os_ok = os == "linux";
    if !os_ok {
        if os == "macos" {
            reasons.push("vLLM has limited macOS support. Linux is recommended.".into());
        } else if os == "windows" {
            reasons.push("vLLM requires WSL2 on Windows.".into());
        }
    }

    // Architecture check
    let arch_ok = arch == "x86_64";
    if !arch_ok && arch == "aarch64" {
        reasons.push("ARM64 has limited CUDA/vLLM support.".into());
    }

    // Python check
    let python_ok = python["sufficient"].as_bool().unwrap_or(false);
    if !python["available"].as_bool().unwrap_or(false) {
        reasons.push("Python is not installed. Install Python 3.10+.".into());
    } else if !python_ok {
        let ver = python["version"].as_str().unwrap_or("unknown");
        reasons.push(format!("Python {} is too old. Python 3.10+ required.", ver));
    }

    // CUDA check
    let cuda_ok = cuda["sufficient"].as_bool().unwrap_or(false);
    if !cuda["available"].as_bool().unwrap_or(false) {
        reasons.push("No NVIDIA GPU detected. CUDA GPU with 10GB+ VRAM required.".into());
    } else if !cuda_ok {
        let mem = cuda["memory_mb"].as_u64().unwrap_or(0);
        reasons.push(format!(
            "GPU has {}MB VRAM. 10GB+ recommended for Voxtral.",
            mem
        ));
    }

    // Overall compatibility
    let compatible = os_ok && arch_ok && python_ok && cuda_ok;

    (compatible, reasons)
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum VoiceProviderId {
    Elevenlabs,
    OpenaiTts,
    GoogleTts,
    Piper,
    Coqui,
    Whisper,
    Groq,
    Deepgram,
    Google,
    Mistral,
    ElevenlabsStt,
    VoxtralLocal,
    WhisperCli,
    SherpaOnnx,
}

/// Static UI metadata for a voice provider (description, key hints, URLs).
struct VoiceProviderMeta {
    description: &'static str,
    key_placeholder: Option<&'static str>,
    key_url: Option<&'static str>,
    key_url_label: Option<&'static str>,
    hint: Option<&'static str>,
}

impl VoiceProviderId {
    fn meta(self) -> VoiceProviderMeta {
        match self {
            // TTS Cloud
            Self::Elevenlabs => VoiceProviderMeta {
                description: "Lowest latency (~75ms), natural voices. Same key enables Scribe STT",
                key_placeholder: Some("API key"),
                key_url: Some("https://elevenlabs.io/app/settings/api-keys"),
                key_url_label: Some("elevenlabs.io"),
                hint: Some(
                    "This API key also enables ElevenLabs Scribe for speech-to-text.",
                ),
            },
            Self::OpenaiTts => VoiceProviderMeta {
                description: "Good quality, shares API key with Whisper STT",
                key_placeholder: Some("sk-..."),
                key_url: Some("https://platform.openai.com/api-keys"),
                key_url_label: Some("platform.openai.com/api-keys"),
                hint: None,
            },
            Self::GoogleTts => VoiceProviderMeta {
                description: "220+ voices, 40+ languages, WaveNet and Neural2 voices",
                key_placeholder: Some("API key"),
                key_url: Some("https://console.cloud.google.com/apis/credentials"),
                key_url_label: Some("console.cloud.google.com"),
                hint: None,
            },
            Self::Piper => VoiceProviderMeta {
                description: "Fast local TTS, commonly used in Home Assistant",
                key_placeholder: None,
                key_url: None,
                key_url_label: None,
                hint: None,
            },
            Self::Coqui => VoiceProviderMeta {
                description: "Open-source deep learning TTS with many voice models",
                key_placeholder: None,
                key_url: None,
                key_url_label: None,
                hint: None,
            },
            // STT Cloud
            Self::Whisper => VoiceProviderMeta {
                description: "Best accuracy, handles accents and background noise",
                key_placeholder: Some("sk-..."),
                key_url: Some("https://platform.openai.com/api-keys"),
                key_url_label: Some("platform.openai.com/api-keys"),
                hint: None,
            },
            Self::Groq => VoiceProviderMeta {
                description: "Ultra-fast Whisper inference on Groq hardware",
                key_placeholder: Some("gsk_..."),
                key_url: Some("https://console.groq.com/keys"),
                key_url_label: Some("console.groq.com/keys"),
                hint: None,
            },
            Self::Deepgram => VoiceProviderMeta {
                description: "Fast and accurate with Nova-3 model",
                key_placeholder: Some("API key"),
                key_url: Some("https://console.deepgram.com/api-keys"),
                key_url_label: Some("console.deepgram.com"),
                hint: None,
            },
            Self::Google => VoiceProviderMeta {
                description: "Supports 125+ languages with Google Speech-to-Text",
                key_placeholder: Some("API key"),
                key_url: Some("https://console.cloud.google.com/apis/credentials"),
                key_url_label: Some("console.cloud.google.com"),
                hint: None,
            },
            Self::Mistral => VoiceProviderMeta {
                description: "Fast Voxtral transcription with 13 language support",
                key_placeholder: Some("API key"),
                key_url: Some("https://console.mistral.ai/api-keys"),
                key_url_label: Some("console.mistral.ai"),
                hint: None,
            },
            Self::ElevenlabsStt => VoiceProviderMeta {
                description: "90+ languages, word timestamps. Same API key as ElevenLabs TTS",
                key_placeholder: Some("API key"),
                key_url: Some("https://elevenlabs.io/app/settings/api-keys"),
                key_url_label: Some("elevenlabs.io"),
                hint: Some(
                    "If you already have ElevenLabs TTS configured, use the same API key here.",
                ),
            },
            // STT Local
            Self::WhisperCli => VoiceProviderMeta {
                description: "Local Whisper inference via whisper-cli",
                key_placeholder: None,
                key_url: None,
                key_url_label: None,
                hint: None,
            },
            Self::SherpaOnnx => VoiceProviderMeta {
                description: "Local offline speech recognition via ONNX runtime",
                key_placeholder: None,
                key_url: None,
                key_url_label: None,
                hint: None,
            },
            Self::VoxtralLocal => VoiceProviderMeta {
                description: "Run Mistral's Voxtral model locally via vLLM server",
                key_placeholder: None,
                key_url: None,
                key_url_label: None,
                hint: None,
            },
        }
    }

    fn parse_tts_list_id(id: &str) -> Option<Self> {
        match id {
            "elevenlabs" => Some(Self::Elevenlabs),
            "openai" | "openai-tts" => Some(Self::OpenaiTts),
            "google" | "google-tts" => Some(Self::GoogleTts),
            "piper" => Some(Self::Piper),
            "coqui" => Some(Self::Coqui),
            _ => None,
        }
    }

    fn parse_stt_list_id(id: &str) -> Option<Self> {
        match id {
            "whisper" => Some(Self::Whisper),
            "groq" => Some(Self::Groq),
            "deepgram" => Some(Self::Deepgram),
            "google" => Some(Self::Google),
            "mistral" => Some(Self::Mistral),
            "elevenlabs" | "elevenlabs-stt" => Some(Self::ElevenlabsStt),
            "voxtral-local" => Some(Self::VoxtralLocal),
            "whisper-cli" => Some(Self::WhisperCli),
            "sherpa-onnx" => Some(Self::SherpaOnnx),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceProviderInfo {
    id: VoiceProviderId,
    name: String,
    #[serde(rename = "type")]
    provider_type: String,
    category: String,
    description: String,
    available: bool,
    enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_url_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    binary_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status_message: Option<String>,
    capabilities: serde_json::Value,
    settings: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    settings_summary: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct VoiceProvidersResponse {
    tts: Vec<VoiceProviderInfo>,
    stt: Vec<VoiceProviderInfo>,
}

/// Detect all available voice providers with their availability status.
async fn detect_voice_providers(config: &moltis_config::MoltisConfig) -> serde_json::Value {
    use secrecy::ExposeSecret;

    // Check for API keys from environment variables
    let env_openai_key = std::env::var("OPENAI_API_KEY").ok();
    let env_elevenlabs_key = std::env::var("ELEVENLABS_API_KEY").ok();
    let env_google_key = std::env::var("GOOGLE_API_KEY")
        .or_else(|_| std::env::var("GOOGLE_CLOUD_API_KEY"))
        .ok();
    let env_groq_key = std::env::var("GROQ_API_KEY").ok();
    let env_deepgram_key = std::env::var("DEEPGRAM_API_KEY").ok();
    let env_mistral_key = std::env::var("MISTRAL_API_KEY").ok();

    // Check for API keys from LLM providers config
    let llm_openai_key = config
        .providers
        .get("openai")
        .and_then(|p| p.api_key.as_ref())
        .map(|k| k.expose_secret().to_string());
    let llm_groq_key = config
        .providers
        .get("groq")
        .and_then(|p| p.api_key.as_ref())
        .map(|k| k.expose_secret().to_string());
    let _llm_deepseek_key = config
        .providers
        .get("deepseek")
        .and_then(|p| p.api_key.as_ref())
        .map(|k| k.expose_secret().to_string());

    // Check for local binaries
    let whisper_cli_available = check_binary_available("whisper-cpp")
        .await
        .or(check_binary_available("whisper").await);
    let piper_available = check_binary_available("piper").await;
    let sherpa_onnx_available = check_binary_available("sherpa-onnx-offline").await;
    let coqui_server_running = check_coqui_server(&config.voice.tts.coqui.endpoint).await;
    let tts_server_binary = check_binary_available("tts-server").await;

    // Build TTS providers list
    let tts_providers = vec![
        build_provider_info(
            VoiceProviderId::Elevenlabs,
            "ElevenLabs",
            "tts",
            "cloud",
            config.voice.tts.elevenlabs.api_key.is_some() || env_elevenlabs_key.is_some(),
            config.voice.tts.provider == "elevenlabs" && config.voice.tts.enabled,
            key_source(
                config.voice.tts.elevenlabs.api_key.is_some(),
                env_elevenlabs_key.is_some(),
                false,
            ),
            None,
            None,
        ),
        build_provider_info(
            VoiceProviderId::OpenaiTts,
            "OpenAI TTS",
            "tts",
            "cloud",
            config.voice.tts.openai.api_key.is_some()
                || env_openai_key.is_some()
                || llm_openai_key.is_some(),
            config.voice.tts.provider == "openai" && config.voice.tts.enabled,
            key_source(
                config.voice.tts.openai.api_key.is_some(),
                env_openai_key.is_some(),
                llm_openai_key.is_some(),
            ),
            None,
            None,
        ),
        build_provider_info(
            VoiceProviderId::GoogleTts,
            "Google Cloud TTS",
            "tts",
            "cloud",
            config.voice.tts.google.api_key.is_some() || env_google_key.is_some(),
            config.voice.tts.provider == "google" && config.voice.tts.enabled,
            key_source(
                config.voice.tts.google.api_key.is_some(),
                env_google_key.is_some(),
                false,
            ),
            None,
            None,
        ),
        build_provider_info(
            VoiceProviderId::Piper,
            "Piper",
            "tts",
            "local",
            piper_available.is_some() && config.voice.tts.piper.model_path.is_some(),
            config.voice.tts.provider == "piper" && config.voice.tts.enabled,
            None,
            piper_available.clone(),
            if piper_available.is_none() {
                Some(
                    "piper binary not found. Install from https://github.com/rhasspy/piper/releases",
                )
            } else if config.voice.tts.piper.model_path.is_none() {
                Some(
                    "model not configured - download voice models from https://rhasspy.github.io/piper-samples/",
                )
            } else {
                None
            },
        ),
        build_provider_info(
            VoiceProviderId::Coqui,
            "Coqui TTS",
            "tts",
            "local",
            coqui_server_running,
            config.voice.tts.provider == "coqui" && config.voice.tts.enabled,
            None,
            tts_server_binary,
            if !coqui_server_running {
                Some("server not running")
            } else {
                None
            },
        ),
    ];

    // Check voxtral local server
    let voxtral_server_running = check_vllm_server(&config.voice.stt.voxtral_local.endpoint).await;

    // Build STT providers list
    let stt_providers = vec![
        build_provider_info(
            VoiceProviderId::Whisper,
            "OpenAI Whisper",
            "stt",
            "cloud",
            config.voice.stt.whisper.api_key.is_some()
                || env_openai_key.is_some()
                || llm_openai_key.is_some(),
            config.voice.stt.provider == VoiceSttProvider::Whisper && config.voice.stt.enabled,
            key_source(
                config.voice.stt.whisper.api_key.is_some(),
                env_openai_key.is_some(),
                llm_openai_key.is_some(),
            ),
            None,
            None,
        ),
        build_provider_info(
            VoiceProviderId::Groq,
            "Groq",
            "stt",
            "cloud",
            config.voice.stt.groq.api_key.is_some()
                || env_groq_key.is_some()
                || llm_groq_key.is_some(),
            config.voice.stt.provider == VoiceSttProvider::Groq && config.voice.stt.enabled,
            key_source(
                config.voice.stt.groq.api_key.is_some(),
                env_groq_key.is_some(),
                llm_groq_key.is_some(),
            ),
            None,
            None,
        ),
        build_provider_info(
            VoiceProviderId::Deepgram,
            "Deepgram",
            "stt",
            "cloud",
            config.voice.stt.deepgram.api_key.is_some() || env_deepgram_key.is_some(),
            config.voice.stt.provider == VoiceSttProvider::Deepgram && config.voice.stt.enabled,
            key_source(
                config.voice.stt.deepgram.api_key.is_some(),
                env_deepgram_key.is_some(),
                false,
            ),
            None,
            None,
        ),
        build_provider_info(
            VoiceProviderId::Google,
            "Google Cloud STT",
            "stt",
            "cloud",
            config.voice.stt.google.api_key.is_some() || env_google_key.is_some(),
            config.voice.stt.provider == VoiceSttProvider::Google && config.voice.stt.enabled,
            key_source(
                config.voice.stt.google.api_key.is_some(),
                env_google_key.is_some(),
                false,
            ),
            None,
            None,
        ),
        build_provider_info(
            VoiceProviderId::Mistral,
            "Mistral (Voxtral)",
            "stt",
            "cloud",
            config.voice.stt.mistral.api_key.is_some() || env_mistral_key.is_some(),
            config.voice.stt.provider == VoiceSttProvider::Mistral && config.voice.stt.enabled,
            key_source(
                config.voice.stt.mistral.api_key.is_some(),
                env_mistral_key.is_some(),
                false,
            ),
            None,
            None,
        ),
        build_provider_info(
            VoiceProviderId::ElevenlabsStt,
            "ElevenLabs Scribe",
            "stt",
            "cloud",
            config.voice.stt.elevenlabs.api_key.is_some()
                || config.voice.tts.elevenlabs.api_key.is_some()
                || env_elevenlabs_key.is_some(),
            config.voice.stt.provider == VoiceSttProvider::ElevenLabs && config.voice.stt.enabled,
            key_source(
                config.voice.stt.elevenlabs.api_key.is_some()
                    || config.voice.tts.elevenlabs.api_key.is_some(),
                env_elevenlabs_key.is_some(),
                false,
            ),
            None,
            None,
        ),
        build_provider_info(
            VoiceProviderId::VoxtralLocal,
            "Voxtral (Local)",
            "stt",
            "local",
            voxtral_server_running,
            config.voice.stt.provider == VoiceSttProvider::VoxtralLocal && config.voice.stt.enabled,
            None,
            None,
            if !voxtral_server_running {
                Some("server not running")
            } else {
                None
            },
        ),
        build_provider_info(
            VoiceProviderId::WhisperCli,
            "whisper.cpp",
            "stt",
            "local",
            whisper_cli_available.is_some() && config.voice.stt.whisper_cli.model_path.is_some(),
            config.voice.stt.provider == VoiceSttProvider::WhisperCli && config.voice.stt.enabled,
            None,
            whisper_cli_available.clone(),
            if whisper_cli_available.is_none() {
                Some(
                    "whisper-cpp binary not found. Install with: brew install whisper-cpp (macOS) or build from https://github.com/ggerganov/whisper.cpp",
                )
            } else if config.voice.stt.whisper_cli.model_path.is_none() {
                Some(
                    "model not configured - download a GGML model from https://huggingface.co/ggerganov/whisper.cpp",
                )
            } else {
                None
            },
        ),
        build_provider_info(
            VoiceProviderId::SherpaOnnx,
            "sherpa-onnx",
            "stt",
            "local",
            sherpa_onnx_available.is_some() && config.voice.stt.sherpa_onnx.model_dir.is_some(),
            config.voice.stt.provider == VoiceSttProvider::SherpaOnnx && config.voice.stt.enabled,
            None,
            sherpa_onnx_available.clone(),
            if sherpa_onnx_available.is_none() {
                Some(
                    "sherpa-onnx binary not found. Download from https://github.com/k2-fsa/sherpa-onnx/releases",
                )
            } else if config.voice.stt.sherpa_onnx.model_dir.is_none() {
                Some(
                    "model not configured - download models from https://github.com/k2-fsa/sherpa-onnx/releases",
                )
            } else {
                None
            },
        ),
    ];

    let tts_with_details = filter_listed_voice_providers(
        tts_providers
            .into_iter()
            .map(|provider| enrich_voice_provider(provider, config))
            .collect::<Vec<_>>(),
        &config.voice.tts.providers,
        VoiceProviderId::parse_tts_list_id,
    );
    let stt_with_details = filter_listed_voice_providers(
        stt_providers
            .into_iter()
            .map(|provider| enrich_voice_provider(provider, config))
            .collect::<Vec<_>>(),
        &config.voice.stt.providers,
        VoiceProviderId::parse_stt_list_id,
    );

    serde_json::to_value(VoiceProvidersResponse {
        tts: tts_with_details,
        stt: stt_with_details,
    })
    .unwrap_or_else(|_| serde_json::json!({ "tts": [], "stt": [] }))
}

fn filter_listed_voice_providers(
    providers: Vec<VoiceProviderInfo>,
    listed_provider_ids: &[String],
    parse_provider_id: fn(&str) -> Option<VoiceProviderId>,
) -> Vec<VoiceProviderInfo> {
    if listed_provider_ids.is_empty() {
        return providers;
    }

    let allowed_ids: Vec<_> = listed_provider_ids
        .iter()
        .filter_map(|id| parse_provider_id(id))
        .collect();

    providers
        .into_iter()
        .filter(|provider| allowed_ids.contains(&provider.id))
        .collect()
}

fn enrich_voice_provider(
    mut provider: VoiceProviderInfo,
    config: &moltis_config::MoltisConfig,
) -> VoiceProviderInfo {
    let (capabilities, settings, summary) = match provider.id {
        VoiceProviderId::OpenaiTts => (
            serde_json::json!({
                "voiceChoices": ["alloy", "echo", "fable", "onyx", "nova", "shimmer"],
                "modelChoices": ["tts-1", "tts-1-hd"],
                "customVoice": true,
                "customModel": true,
            }),
            serde_json::json!({
                "voice": config.voice.tts.openai.voice,
                "model": config.voice.tts.openai.model,
            }),
            format_voice_summary(
                config.voice.tts.openai.voice.clone(),
                config.voice.tts.openai.model.clone(),
            ),
        ),
        VoiceProviderId::Elevenlabs => (
            serde_json::json!({
                "voiceId": true,
                "modelChoices": ["eleven_flash_v2_5", "eleven_turbo_v2_5", "eleven_multilingual_v2"],
                "customVoice": true,
                "customModel": true,
            }),
            serde_json::json!({
                "voiceId": config.voice.tts.elevenlabs.voice_id,
                "model": config.voice.tts.elevenlabs.model,
            }),
            format_voice_summary(
                config.voice.tts.elevenlabs.voice_id.clone(),
                config.voice.tts.elevenlabs.model.clone(),
            ),
        ),
        VoiceProviderId::GoogleTts => (
            serde_json::json!({
                "languageChoices": ["en-US", "en-GB", "fr-FR", "de-DE", "es-ES", "it-IT", "pt-BR", "ja-JP"],
                "exampleVoices": [
                    "en-US-Neural2-A", "en-US-Neural2-C", "en-GB-Neural2-A", "en-GB-Neural2-B",
                    "fr-FR-Neural2-A", "de-DE-Neural2-B"
                ],
                "customVoice": true,
                "customLanguage": true,
            }),
            serde_json::json!({
                "voice": config.voice.tts.google.voice,
                "languageCode": config.voice.tts.google.language_code,
            }),
            format_voice_summary(
                config.voice.tts.google.voice.clone(),
                config.voice.tts.google.language_code.clone(),
            ),
        ),
        VoiceProviderId::Coqui => (
            serde_json::json!({
                "speaker": true,
                "language": true,
                "customSpeaker": true,
                "customLanguage": true,
            }),
            serde_json::json!({
                "speaker": config.voice.tts.coqui.speaker,
                "language": config.voice.tts.coqui.language,
                "model": config.voice.tts.coqui.model,
            }),
            format_voice_summary(
                config.voice.tts.coqui.speaker.clone(),
                config.voice.tts.coqui.language.clone(),
            ),
        ),
        VoiceProviderId::Piper => (
            serde_json::json!({
                "speakerId": true,
                "customModelPath": true,
            }),
            serde_json::json!({
                "speakerId": config.voice.tts.piper.speaker_id,
                "modelPath": config.voice.tts.piper.model_path,
            }),
            format_voice_summary(
                config
                    .voice
                    .tts
                    .piper
                    .speaker_id
                    .map(|s| format!("speaker {}", s)),
                None,
            ),
        ),
        _ => (serde_json::json!({}), serde_json::json!({}), None),
    };

    provider.capabilities = capabilities;
    provider.settings = settings;
    provider.settings_summary = summary;
    provider
}

fn format_voice_summary(primary: Option<String>, secondary: Option<String>) -> Option<String> {
    match (primary, secondary) {
        (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() => Some(format!("{} · {}", a, b)),
        (Some(a), _) if !a.is_empty() => Some(a),
        (_, Some(b)) if !b.is_empty() => Some(b),
        _ => None,
    }
}

#[derive(Debug, serde::Deserialize)]
struct ElevenLabsVoiceListResponse {
    voices: Vec<ElevenLabsVoice>,
}

#[derive(Debug, serde::Deserialize)]
struct ElevenLabsVoice {
    voice_id: String,
    name: String,
}

#[derive(Debug, serde::Deserialize)]
struct ElevenLabsModel {
    model_id: String,
    name: String,
    #[serde(default)]
    can_do_text_to_speech: Option<bool>,
}

async fn fetch_elevenlabs_catalog(config: &moltis_config::MoltisConfig) -> serde_json::Value {
    use secrecy::ExposeSecret;

    let fallback_models = vec![
        serde_json::json!({ "id": "eleven_flash_v2_5", "name": "Eleven Flash v2.5" }),
        serde_json::json!({ "id": "eleven_turbo_v2_5", "name": "Eleven Turbo v2.5" }),
        serde_json::json!({ "id": "eleven_multilingual_v2", "name": "Eleven Multilingual v2" }),
        serde_json::json!({ "id": "eleven_monolingual_v1", "name": "Eleven Monolingual v1" }),
    ];

    let api_key = config
        .voice
        .tts
        .elevenlabs
        .api_key
        .as_ref()
        .or(config.voice.stt.elevenlabs.api_key.as_ref())
        .map(|k| k.expose_secret().to_string())
        .or_else(|| std::env::var("ELEVENLABS_API_KEY").ok());

    let Some(api_key) = api_key else {
        return serde_json::json!({
            "voices": [],
            "models": fallback_models,
            "warning": "No ElevenLabs API key configured. Showing known model suggestions only.",
        });
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build();
    let Ok(client) = client else {
        return serde_json::json!({ "voices": [], "models": fallback_models });
    };

    let voices_req = client
        .get("https://api.elevenlabs.io/v1/voices")
        .header("xi-api-key", &api_key)
        .send();
    let models_req = client
        .get("https://api.elevenlabs.io/v1/models")
        .header("xi-api-key", &api_key)
        .send();

    let (voices_res, models_res) = tokio::join!(voices_req, models_req);

    let voices = match voices_res {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<ElevenLabsVoiceListResponse>().await {
                Ok(body) => body
                    .voices
                    .into_iter()
                    .map(|v| serde_json::json!({ "id": v.voice_id, "name": v.name }))
                    .collect::<Vec<_>>(),
                Err(_) => Vec::new(),
            }
        },
        _ => Vec::new(),
    };

    let models = match models_res {
        Ok(resp) if resp.status().is_success() => match resp.json::<Vec<ElevenLabsModel>>().await {
            Ok(body) => {
                let parsed: Vec<_> = body
                    .into_iter()
                    .filter(|m| m.can_do_text_to_speech.unwrap_or(true))
                    .map(|m| serde_json::json!({ "id": m.model_id, "name": m.name }))
                    .collect();
                if parsed.is_empty() {
                    fallback_models.clone()
                } else {
                    parsed
                }
            },
            Err(_) => fallback_models.clone(),
        },
        _ => fallback_models.clone(),
    };

    serde_json::json!({
        "voices": voices,
        "models": models,
    })
}

fn build_provider_info(
    id: VoiceProviderId,
    name: &str,
    provider_type: &str,
    category: &str,
    available: bool,
    enabled: bool,
    key_source: Option<&str>,
    binary_path: Option<String>,
    status_message: Option<&str>,
) -> VoiceProviderInfo {
    let meta = id.meta();
    VoiceProviderInfo {
        id,
        name: name.to_string(),
        provider_type: provider_type.to_string(),
        category: category.to_string(),
        description: meta.description.to_string(),
        available,
        enabled,
        key_source: key_source.map(str::to_string),
        key_placeholder: meta.key_placeholder.map(str::to_string),
        key_url: meta.key_url.map(str::to_string),
        key_url_label: meta.key_url_label.map(str::to_string),
        hint: meta.hint.map(str::to_string),
        binary_path,
        status_message: status_message.map(str::to_string),
        capabilities: serde_json::json!({}),
        settings: serde_json::json!({}),
        settings_summary: None,
    }
}

fn key_source(in_config: bool, in_env: bool, in_llm_provider: bool) -> Option<&'static str> {
    if in_config {
        Some("config")
    } else if in_env {
        Some("env")
    } else if in_llm_provider {
        Some("llm_provider")
    } else {
        None
    }
}

fn apply_voice_provider_settings(
    cfg: &mut moltis_config::MoltisConfig,
    provider: &str,
    params: &serde_json::Value,
) {
    let get_string = |key: &str| -> Option<String> {
        params
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
    };

    match provider {
        "openai" | "openai-tts" => {
            if let Some(voice) = get_string("voice") {
                cfg.voice.tts.openai.voice = Some(voice);
            }
            if let Some(model) = get_string("model") {
                cfg.voice.tts.openai.model = Some(model);
            }
        },
        "elevenlabs" => {
            if let Some(voice_id) = get_string("voiceId") {
                cfg.voice.tts.elevenlabs.voice_id = Some(voice_id);
            }
            if let Some(model) = get_string("model") {
                cfg.voice.tts.elevenlabs.model = Some(model);
            }
        },
        "google" | "google-tts" => {
            if let Some(voice) = get_string("voice") {
                cfg.voice.tts.google.voice = Some(voice);
            }
            if let Some(language_code) = get_string("languageCode") {
                cfg.voice.tts.google.language_code = Some(language_code);
            }
        },
        "coqui" => {
            if let Some(speaker) = get_string("speaker") {
                cfg.voice.tts.coqui.speaker = Some(speaker);
            }
            if let Some(language) = get_string("language") {
                cfg.voice.tts.coqui.language = Some(language);
            }
            if let Some(model) = get_string("model") {
                cfg.voice.tts.coqui.model = Some(model);
            }
        },
        "piper" => {
            if let Some(model_path) = get_string("modelPath") {
                cfg.voice.tts.piper.model_path = Some(model_path);
            }
            if let Some(speaker_id) = params
                .get("speakerId")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
            {
                cfg.voice.tts.piper.speaker_id = Some(speaker_id);
            }
        },
        _ => {},
    }
}

async fn check_binary_available(name: &str) -> Option<String> {
    // Try to find the binary in PATH
    if let Ok(output) = tokio::process::Command::new("which")
        .arg(name)
        .output()
        .await
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Some(path);
        }
    }
    None
}

/// Check if Coqui TTS server is running.
async fn check_coqui_server(endpoint: &str) -> bool {
    // Try to connect to the server's health endpoint
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap_or_default();

    // Coqui TTS server responds to GET /
    if let Ok(resp) = client.get(endpoint).send().await {
        return resp.status().is_success();
    }
    false
}

/// Check if vLLM server is running (for Voxtral local).
async fn check_vllm_server(endpoint: &str) -> bool {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap_or_default();

    // vLLM exposes /health endpoint
    let health_url = format!("{}/health", endpoint.trim_end_matches('/'));
    if let Ok(resp) = client.get(&health_url).send().await {
        return resp.status().is_success();
    }
    false
}

/// Toggle a voice provider on/off by updating the config file.
fn toggle_voice_provider(
    provider: &str,
    enabled: bool,
    provider_type: &str,
) -> Result<(), anyhow::Error> {
    moltis_config::update_config(|cfg| {
        match provider_type {
            "tts" => {
                if enabled {
                    // Map provider id to config provider name
                    let config_provider = match provider {
                        "openai-tts" => "openai",
                        "google-tts" => "google",
                        other => other,
                    };
                    cfg.voice.tts.provider = config_provider.to_string();
                    cfg.voice.tts.enabled = true;
                } else if cfg.voice.tts.provider == provider
                    || (provider == "openai-tts" && cfg.voice.tts.provider == "openai")
                    || (provider == "google-tts" && cfg.voice.tts.provider == "google")
                {
                    cfg.voice.tts.enabled = false;
                }
            },
            "stt" => {
                let stt_provider = VoiceSttProvider::parse(provider);
                if enabled {
                    if let Some(provider_id) = stt_provider {
                        cfg.voice.stt.provider = provider_id;
                        cfg.voice.stt.enabled = true;
                    }
                } else if stt_provider
                    .is_some_and(|provider_id| cfg.voice.stt.provider == provider_id)
                {
                    cfg.voice.stt.enabled = false;
                }
            },
            _ => {},
        }
    })?;
    Ok(())
}

/// Re-run hook discovery, rebuild the registry, and broadcast the update.
async fn reload_hooks(state: &Arc<GatewayState>) {
    let disabled = state.disabled_hooks.read().await.clone();
    let session_store = state.services.session_store.as_ref();
    let (new_registry, new_info) =
        crate::server::discover_and_build_hooks(&disabled, session_store).await;

    *state.hook_registry.write().await = new_registry;
    *state.discovered_hooks.write().await = new_info.clone();

    // Broadcast hooks.status event so connected UIs auto-refresh.
    broadcast(
        state,
        "hooks.status",
        serde_json::json!({ "hooks": new_info }),
        BroadcastOpts::default(),
    )
    .await;
}

/// Persist the disabled hooks set to `data_dir/disabled_hooks.json`.
async fn persist_disabled_hooks(state: &Arc<GatewayState>) {
    let disabled = state.disabled_hooks.read().await;
    let path = moltis_config::data_dir().join("disabled_hooks.json");
    let json = serde_json::to_string_pretty(&*disabled).unwrap_or_default();
    if let Err(e) = std::fs::write(&path, json) {
        warn!("failed to persist disabled hooks: {e}");
    }
}

/// Load the disabled hooks set from `data_dir/disabled_hooks.json`.
pub(crate) fn load_disabled_hooks() -> std::collections::HashSet<String> {
    let path = moltis_config::data_dir().join("disabled_hooks.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scopes(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    fn test_voice_provider(id: VoiceProviderId) -> VoiceProviderInfo {
        let meta = id.meta();
        VoiceProviderInfo {
            id,
            name: String::new(),
            provider_type: String::new(),
            category: String::new(),
            description: meta.description.to_string(),
            available: false,
            enabled: false,
            key_source: None,
            key_placeholder: meta.key_placeholder.map(str::to_string),
            key_url: meta.key_url.map(str::to_string),
            key_url_label: meta.key_url_label.map(str::to_string),
            hint: meta.hint.map(str::to_string),
            binary_path: None,
            status_message: None,
            capabilities: serde_json::json!({}),
            settings: serde_json::json!({}),
            settings_summary: None,
        }
    }

    #[test]
    fn parse_voice_provider_list_aliases() {
        assert_eq!(
            VoiceProviderId::parse_tts_list_id("openai"),
            Some(VoiceProviderId::OpenaiTts)
        );
        assert_eq!(
            VoiceProviderId::parse_tts_list_id("google-tts"),
            Some(VoiceProviderId::GoogleTts)
        );
        assert_eq!(
            VoiceProviderId::parse_stt_list_id("elevenlabs"),
            Some(VoiceProviderId::ElevenlabsStt)
        );
        assert_eq!(
            VoiceProviderId::parse_stt_list_id("sherpa-onnx"),
            Some(VoiceProviderId::SherpaOnnx)
        );
    }

    #[test]
    fn filter_listed_voice_providers_keeps_all_when_list_is_empty() {
        let filtered = filter_listed_voice_providers(
            vec![
                test_voice_provider(VoiceProviderId::OpenaiTts),
                test_voice_provider(VoiceProviderId::GoogleTts),
            ],
            &[],
            VoiceProviderId::parse_tts_list_id,
        );
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filter_listed_voice_providers_filters_tts_ids() {
        let filtered = filter_listed_voice_providers(
            vec![
                test_voice_provider(VoiceProviderId::OpenaiTts),
                test_voice_provider(VoiceProviderId::GoogleTts),
                test_voice_provider(VoiceProviderId::Piper),
            ],
            &["openai".to_string(), "piper".to_string()],
            VoiceProviderId::parse_tts_list_id,
        );
        let ids: Vec<_> = filtered.into_iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![
            VoiceProviderId::OpenaiTts,
            VoiceProviderId::Piper
        ]);
    }

    #[test]
    fn senders_list_requires_read() {
        // With read scope → authorized
        assert!(
            authorize_method(
                "channels.senders.list",
                "operator",
                &scopes(&["operator.read"])
            )
            .is_none()
        );
        // Without read or write → denied
        assert!(authorize_method("channels.senders.list", "operator", &scopes(&[])).is_some());
    }

    #[test]
    fn senders_approve_requires_write() {
        assert!(
            authorize_method(
                "channels.senders.approve",
                "operator",
                &scopes(&["operator.write"])
            )
            .is_none()
        );
        assert!(
            authorize_method(
                "channels.senders.approve",
                "operator",
                &scopes(&["operator.read"])
            )
            .is_some()
        );
    }

    #[test]
    fn senders_deny_requires_write() {
        assert!(
            authorize_method(
                "channels.senders.deny",
                "operator",
                &scopes(&["operator.write"])
            )
            .is_none()
        );
        assert!(
            authorize_method(
                "channels.senders.deny",
                "operator",
                &scopes(&["operator.read"])
            )
            .is_some()
        );
    }

    #[test]
    fn admin_scope_allows_all_sender_methods() {
        for method in &[
            "channels.senders.list",
            "channels.senders.approve",
            "channels.senders.deny",
        ] {
            assert!(
                authorize_method(method, "operator", &scopes(&["operator.admin"])).is_none(),
                "admin should authorize {method}"
            );
        }
    }

    #[test]
    fn node_role_denied_sender_methods() {
        for method in &[
            "channels.senders.list",
            "channels.senders.approve",
            "channels.senders.deny",
        ] {
            assert!(
                authorize_method(method, "node", &scopes(&["operator.admin"])).is_some(),
                "node role should be denied for {method}"
            );
        }
    }

    #[test]
    fn identity_get_requires_read() {
        // Read scope is sufficient for get
        assert!(
            authorize_method(
                "agent.identity.get",
                "operator",
                &scopes(&["operator.read"])
            )
            .is_none()
        );
        // No scope → denied
        assert!(authorize_method("agent.identity.get", "operator", &scopes(&[])).is_some());
    }

    #[test]
    fn identity_update_requires_write() {
        // Write scope → authorized
        assert!(
            authorize_method(
                "agent.identity.update",
                "operator",
                &scopes(&["operator.write"])
            )
            .is_none()
        );
        // Read-only scope → denied (these methods modify config)
        assert!(
            authorize_method(
                "agent.identity.update",
                "operator",
                &scopes(&["operator.read"])
            )
            .is_some()
        );
    }

    #[test]
    fn identity_update_soul_requires_write() {
        // Write scope → authorized
        assert!(
            authorize_method(
                "agent.identity.update_soul",
                "operator",
                &scopes(&["operator.write"])
            )
            .is_none()
        );
        // Read-only scope → denied (these methods modify config)
        assert!(
            authorize_method(
                "agent.identity.update_soul",
                "operator",
                &scopes(&["operator.read"])
            )
            .is_some()
        );
    }

    #[test]
    fn cron_read_methods_require_read() {
        for method in &["cron.list", "cron.status", "cron.runs"] {
            assert!(
                authorize_method(method, "operator", &scopes(&["operator.read"])).is_none(),
                "read scope should authorize {method}"
            );
            assert!(
                authorize_method(method, "operator", &scopes(&[])).is_some(),
                "no scope should deny {method}"
            );
        }
    }

    #[test]
    fn cron_write_methods_require_write() {
        for method in &["cron.add", "cron.update", "cron.remove", "cron.run"] {
            assert!(
                authorize_method(method, "operator", &scopes(&["operator.write"])).is_none(),
                "write scope should authorize {method}"
            );
            assert!(
                authorize_method(method, "operator", &scopes(&["operator.read"])).is_some(),
                "read-only scope should deny {method}"
            );
        }
    }

    #[test]
    fn hooks_list_requires_read() {
        assert!(authorize_method("hooks.list", "operator", &scopes(&["operator.read"])).is_none());
        assert!(authorize_method("hooks.list", "operator", &scopes(&[])).is_some());
    }

    #[test]
    fn hooks_write_methods_require_write() {
        for method in &[
            "hooks.enable",
            "hooks.disable",
            "hooks.save",
            "hooks.reload",
        ] {
            assert!(
                authorize_method(method, "operator", &scopes(&["operator.write"])).is_none(),
                "write scope should authorize {method}"
            );
            assert!(
                authorize_method(method, "operator", &scopes(&["operator.read"])).is_some(),
                "read-only scope should deny {method}"
            );
        }
    }

    #[test]
    fn model_probe_params_include_provider_when_present() {
        let params = model_probe_params(Some("github-copilot"));
        assert_eq!(params["background"], serde_json::json!(true));
        assert_eq!(params["reason"], serde_json::json!("provider_connected"));
        assert_eq!(params["provider"], serde_json::json!("github-copilot"));
    }

    #[test]
    fn model_probe_params_omit_provider_when_missing() {
        let params = model_probe_params(None);
        assert_eq!(params["background"], serde_json::json!(true));
        assert_eq!(params["reason"], serde_json::json!("provider_connected"));
        assert!(params.get("provider").is_none());
    }

    #[test]
    fn model_probe_params_omit_provider_when_blank() {
        let params = model_probe_params(Some("   "));
        assert!(params.get("provider").is_none());
    }
}
