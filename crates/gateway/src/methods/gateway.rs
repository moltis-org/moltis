use crate::broadcast::{BroadcastOpts, broadcast};

use super::MethodRegistry;

pub(super) fn register(reg: &mut MethodRegistry) {
    // health
    reg.register(
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
    reg.register(
        "status",
        Box::new(|ctx| {
            Box::pin(async move {
                let inner = ctx.state.inner.read().await;
                let nodes = &inner.nodes;
                Ok(serde_json::json!({
                    "version": ctx.state.version,
                    "hostname": ctx.state.hostname,
                    "connections": inner.clients.len(),
                    "uptimeMs": ctx.state.uptime_ms(),
                    "nodes": nodes.count(),
                    "hasMobileNode": nodes.has_mobile_node(),
                }))
            })
        }),
    );

    // system-presence
    reg.register(
        "system-presence",
        Box::new(|ctx| {
            Box::pin(async move {
                let inner = ctx.state.inner.read().await;

                let client_list: Vec<_> = inner
                    .clients
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

                let node_list: Vec<_> = inner
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
    reg.register(
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
    reg.register(
        "last-heartbeat",
        Box::new(|ctx| {
            Box::pin(async move {
                let inner = ctx.state.inner.read().await;
                if let Some(client) = inner.clients.get(&ctx.client_conn_id) {
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
    reg.register(
        "set-heartbeats",
        Box::new(|ctx| {
            Box::pin(async move {
                if let Some(client) = ctx
                    .state
                    .inner
                    .write()
                    .await
                    .clients
                    .get_mut(&ctx.client_conn_id)
                {
                    client.touch();
                }
                Ok(serde_json::json!({}))
            })
        }),
    );
}
