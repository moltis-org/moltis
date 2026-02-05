use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use tracing::{debug, warn};

use moltis_protocol::{ErrorShape, ResponseFrame, error_codes};

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
    "agents.list",
    "agent.identity.get",
    "agent.identity.update",
    "agent.identity.update_soul",
    "skills.list",
    "skills.status",
    "skills.repos.list",
    "voicewake.get",
    "sessions.list",
    "sessions.preview",
    "sessions.search",
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
];

const WRITE_METHODS: &[&str] = &[
    "send",
    "agent",
    "agent.wait",
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
    "providers.save_key",
    "providers.remove_key",
    "providers.oauth.start",
    "providers.local.configure",
    "providers.local.configure_custom",
    "channels.add",
    "channels.remove",
    "channels.update",
    "channels.senders.approve",
    "channels.senders.deny",
    "sessions.switch",
    "projects.upsert",
    "projects.delete",
    "projects.detect",
    "skills.install",
    "skills.remove",
    "skills.repos.remove",
    "skills.skill.enable",
    "skills.skill.disable",
    "skills.install_dep",
    "plugins.install",
    "plugins.remove",
    "plugins.repos.remove",
    "plugins.skill.enable",
    "plugins.skill.disable",
    "mcp.add",
    "mcp.remove",
    "mcp.enable",
    "mcp.disable",
    "mcp.restart",
    "mcp.update",
    "heartbeat.update",
    "heartbeat.run",
    "voice.config.save_key",
    "voice.config.remove_key",
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
                        let prompt = moltis_cron::heartbeat::resolve_heartbeat_prompt(
                            patch.prompt.as_deref(),
                        );
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
                    ctx.state
                        .services
                        .skills
                        .install(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
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

        // Plugins
        self.register(
            "plugins.install",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .plugins
                        .install(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "plugins.remove",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .plugins
                        .remove(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "plugins.repos.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .plugins
                        .repos_list()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "plugins.repos.remove",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .plugins
                        .repos_remove(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "plugins.skill.enable",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .plugins
                        .skill_enable(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "plugins.skill.disable",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .plugins
                        .skill_disable(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
                })
            }),
        );
        self.register(
            "plugins.skill.detail",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .plugins
                        .skill_detail(ctx.params.clone())
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
                    ctx.state
                        .services
                        .provider_setup
                        .oauth_start(ctx.params.clone())
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))
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
                                "whisper_cli_configured": config.voice.stt.whisper_cli.model_path.is_some(),
                                "sherpa_onnx_configured": config.voice.stt.sherpa_onnx.model_dir.is_some(),
                            },
                        }))
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

                        moltis_config::update_config(|cfg| match provider {
                            // TTS providers
                            "elevenlabs" => {
                                cfg.voice.tts.elevenlabs.api_key =
                                    Some(Secret::new(api_key.to_string()));
                            },
                            "openai" => {
                                cfg.voice.tts.openai.api_key =
                                    Some(Secret::new(api_key.to_string()));
                            },
                            // STT providers
                            "whisper" => {
                                cfg.voice.stt.whisper.api_key =
                                    Some(Secret::new(api_key.to_string()));
                            },
                            "groq" => {
                                cfg.voice.stt.groq.api_key = Some(Secret::new(api_key.to_string()));
                            },
                            "deepgram" => {
                                cfg.voice.stt.deepgram.api_key =
                                    Some(Secret::new(api_key.to_string()));
                            },
                            "google" => {
                                cfg.voice.stt.google.api_key =
                                    Some(Secret::new(api_key.to_string()));
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
                            serde_json::json!({ "provider": provider }),
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scopes(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
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
}
