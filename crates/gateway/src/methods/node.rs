use std::time::Duration;

use moltis_protocol::{ErrorShape, error_codes};

use crate::broadcast::{BroadcastOpts, broadcast};

use super::MethodRegistry;

pub(super) fn register(reg: &mut MethodRegistry) {
    // node.list
    reg.register(
        "node.list",
        Box::new(|ctx| {
            Box::pin(async move {
                let inner = ctx.state.inner.read().await;
                let list: Vec<_> = inner
                    .nodes
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
    reg.register(
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
                let inner = ctx.state.inner.read().await;
                let node = inner
                    .nodes
                    .get(node_id)
                    .ok_or_else(|| ErrorShape::new(error_codes::UNAVAILABLE, "node not found"))?;
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
    reg.register(
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
                let mut inner = ctx.state.inner.write().await;
                inner
                    .nodes
                    .rename(node_id, name)
                    .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e))?;
                Ok(serde_json::json!({}))
            })
        }),
    );

    // node.invoke: forward an RPC request to a connected node
    reg.register(
        "node.invoke",
        Box::new(|ctx| {
            Box::pin(async move {
                let node_id = ctx
                    .params
                    .get("nodeId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ErrorShape::new(error_codes::INVALID_REQUEST, "missing nodeId"))?
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
                    let inner = ctx.state.inner.read().await;
                    let node = inner.nodes.get(&node_id).ok_or_else(|| {
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
                let event_json = serde_json::to_string(&invoke_event)
                    .map_err(|e| ErrorShape::new(error_codes::INVALID_REQUEST, e.to_string()))?;

                {
                    let inner = ctx.state.inner.read().await;
                    let node_client = inner.clients.get(&conn_id).ok_or_else(|| {
                        ErrorShape::new(error_codes::UNAVAILABLE, "node connection lost")
                    })?;
                    if !node_client.send(&event_json) {
                        return Err(ErrorShape::new(
                            error_codes::UNAVAILABLE,
                            "node send failed",
                        ));
                    }
                }

                // Set up a oneshot for the result with a timeout.
                let (tx, rx) = tokio::sync::oneshot::channel();
                {
                    let mut inner = ctx.state.inner.write().await;
                    inner
                        .pending_invokes
                        .insert(invoke_id.clone(), crate::state::PendingInvoke {
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
                        ctx.state
                            .inner
                            .write()
                            .await
                            .pending_invokes
                            .remove(&invoke_id);
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
    reg.register(
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

                let pending = ctx
                    .state
                    .inner
                    .write()
                    .await
                    .pending_invokes
                    .remove(invoke_id);
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
    reg.register(
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

    // location.result: browser returns the result of a geolocation request
    reg.register(
        "location.result",
        Box::new(|ctx| {
            Box::pin(async move {
                let request_id = ctx
                    .params
                    .get("requestId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, "missing requestId")
                    })?;

                // Build the result value to send through the pending invoke channel.
                let result = if let Some(loc) = ctx.params.get("location") {
                    // Success: cache the location and persist to USER.md.
                    if let (Some(lat), Some(lon)) = (
                        loc.get("latitude").and_then(|v| v.as_f64()),
                        loc.get("longitude").and_then(|v| v.as_f64()),
                    ) {
                        let geo = moltis_config::GeoLocation::now(lat, lon, None);
                        ctx.state.inner.write().await.cached_location = Some(geo.clone());

                        // Persist to USER.md (best-effort).
                        let mut user = moltis_config::load_user().unwrap_or_default();
                        user.location = Some(geo);
                        if let Err(e) = moltis_config::save_user(&user) {
                            tracing::warn!(error = %e, "failed to persist location to USER.md");
                        }
                    }
                    serde_json::json!({ "location": ctx.params.get("location") })
                } else {
                    // Error (permission denied, timeout, etc.)
                    serde_json::json!({ "error": ctx.params.get("error") })
                };

                let pending = ctx
                    .state
                    .inner
                    .write()
                    .await
                    .pending_invokes
                    .remove(request_id);
                if let Some(invoke) = pending {
                    let _ = invoke.sender.send(result);
                    Ok(serde_json::json!({}))
                } else {
                    Err(ErrorShape::new(
                        error_codes::INVALID_REQUEST,
                        "no pending location request for this id",
                    ))
                }
            })
        }),
    );
}
