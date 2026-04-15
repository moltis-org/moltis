use std::time::Duration;

use moltis_protocol::{ErrorShape, error_codes};

use crate::{
    auth::{SshAuthMode, SshResolvedTarget, SshTargetEntry},
    broadcast::{BroadcastOpts, broadcast},
};

use super::MethodRegistry;

fn configured_legacy_ssh_target() -> Option<String> {
    let config = moltis_config::discover_and_load();
    config
        .tools
        .exec
        .ssh_target
        .map(|target| target.trim().to_string())
        .filter(|target| !target.is_empty())
}

fn ssh_summary_json(target: &str) -> serde_json::Value {
    serde_json::json!({
        "nodeId": crate::node_exec::ssh_node_id(target),
        "displayName": format!("SSH: {target}"),
        "platform": "ssh",
        "version": serde_json::Value::Null,
        "capabilities": ["system.run"],
        "commands": ["system.run"],
        "remoteIp": serde_json::Value::Null,
        "telemetry": {
            "memTotal": serde_json::Value::Null,
            "memAvailable": serde_json::Value::Null,
            "cpuCount": serde_json::Value::Null,
            "cpuUsage": serde_json::Value::Null,
            "uptimeSecs": serde_json::Value::Null,
            "services": ["ssh"],
            "diskTotal": serde_json::Value::Null,
            "diskAvailable": serde_json::Value::Null,
            "runtimes": [],
            "stale": false,
        },
        "providers": [],
    })
}

fn ssh_target_summary_json(target: &SshTargetEntry) -> serde_json::Value {
    let auth_service = match target.auth_mode {
        SshAuthMode::System => "ssh-system",
        SshAuthMode::Managed => "ssh-managed",
    };
    serde_json::json!({
        "nodeId": format!("ssh:target:{}", target.id),
        "displayName": format!("SSH: {}", target.label),
        "platform": "ssh",
        "version": serde_json::Value::Null,
        "capabilities": ["system.run"],
        "commands": ["system.run"],
        "remoteIp": serde_json::Value::Null,
        "hostPinned": target.known_host.is_some(),
        "telemetry": {
            "memTotal": serde_json::Value::Null,
            "memAvailable": serde_json::Value::Null,
            "cpuCount": serde_json::Value::Null,
            "cpuUsage": serde_json::Value::Null,
            "uptimeSecs": serde_json::Value::Null,
            "services": ["ssh", auth_service],
            "diskTotal": serde_json::Value::Null,
            "diskAvailable": serde_json::Value::Null,
            "runtimes": [],
            "stale": false,
        },
        "providers": [],
    })
}

fn ssh_detail_json(target: &str) -> serde_json::Value {
    serde_json::json!({
        "nodeId": crate::node_exec::ssh_node_id(target),
        "displayName": format!("SSH: {target}"),
        "platform": "ssh",
        "version": serde_json::Value::Null,
        "capabilities": ["system.run"],
        "commands": ["system.run"],
        "permissions": [],
        "pathEnv": serde_json::Value::Null,
        "remoteIp": serde_json::Value::Null,
        "connectedAt": serde_json::Value::Null,
        "telemetry": {
            "memTotal": serde_json::Value::Null,
            "memAvailable": serde_json::Value::Null,
            "cpuCount": serde_json::Value::Null,
            "cpuUsage": serde_json::Value::Null,
            "uptimeSecs": serde_json::Value::Null,
            "services": ["ssh"],
            "diskTotal": serde_json::Value::Null,
            "diskAvailable": serde_json::Value::Null,
            "runtimes": [],
            "stale": false,
        },
        "providers": [],
    })
}

fn ssh_target_detail_json(target: &SshResolvedTarget) -> serde_json::Value {
    let auth_service = match target.auth_mode {
        SshAuthMode::System => "ssh-system",
        SshAuthMode::Managed => "ssh-managed",
    };
    serde_json::json!({
        "nodeId": target.node_id,
        "displayName": format!("SSH: {}", target.label),
        "platform": "ssh",
        "version": serde_json::Value::Null,
        "capabilities": ["system.run"],
        "commands": ["system.run"],
        "permissions": [],
        "pathEnv": serde_json::Value::Null,
        "remoteIp": serde_json::Value::Null,
        "hostPinned": target.known_host.is_some(),
        "connectedAt": serde_json::Value::Null,
        "telemetry": {
            "memTotal": serde_json::Value::Null,
            "memAvailable": serde_json::Value::Null,
            "cpuCount": serde_json::Value::Null,
            "cpuUsage": serde_json::Value::Null,
            "uptimeSecs": serde_json::Value::Null,
            "services": ["ssh", auth_service],
            "diskTotal": serde_json::Value::Null,
            "diskAvailable": serde_json::Value::Null,
            "runtimes": [],
            "stale": false,
        },
        "providers": [],
    })
}

pub(super) fn register(reg: &mut MethodRegistry) {
    // node.list
    reg.register(
        "node.list",
        Box::new(|ctx| {
            Box::pin(async move {
                let inner = ctx.state.inner.read().await;
                let mut list: Vec<_> = inner
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
                            "telemetry": {
                                "memTotal": n.mem_total,
                                "memAvailable": n.mem_available,
                                "cpuCount": n.cpu_count,
                                "cpuUsage": n.cpu_usage,
                                "uptimeSecs": n.uptime_secs,
                                "services": n.services,
                                "diskTotal": n.disk_total,
                                "diskAvailable": n.disk_available,
                                "runtimes": n.runtimes,
                                "stale": n.last_telemetry.is_some_and(
                                    |t| t.elapsed() > Duration::from_secs(120),
                                ),
                            },
                            "providers": n.providers.iter().map(|p| {
                                serde_json::json!({
                                    "provider": p.provider,
                                    "models": p.models,
                                })
                            }).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                drop(inner);
                if let Some(store) = ctx.state.credential_store.as_ref() {
                    match store.list_ssh_targets().await {
                        Ok(targets) => {
                            for target in targets {
                                list.push(ssh_target_summary_json(&target));
                            }
                        },
                        Err(error) => tracing::warn!(%error, "failed to list managed ssh targets"),
                    }
                }
                if let Some(target) = configured_legacy_ssh_target() {
                    list.push(ssh_summary_json(&target));
                }
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
                if let Some(store) = ctx.state.credential_store.as_ref() {
                    match store.resolve_ssh_target(node_id).await {
                        Ok(Some(target)) => return Ok(ssh_target_detail_json(&target)),
                        Ok(None) => {},
                        Err(error) => {
                            tracing::warn!(%error, node_id, "failed to resolve managed ssh target")
                        },
                    }
                }
                if let Some(target) = configured_legacy_ssh_target()
                    && crate::node_exec::ssh_target_matches(node_id, &target)
                {
                    return Ok(ssh_detail_json(&target));
                }
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
                    "telemetry": {
                        "memTotal": node.mem_total,
                        "memAvailable": node.mem_available,
                        "cpuCount": node.cpu_count,
                        "cpuUsage": node.cpu_usage,
                        "uptimeSecs": node.uptime_secs,
                        "services": node.services,
                        "diskTotal": node.disk_total,
                        "diskAvailable": node.disk_available,
                        "runtimes": node.runtimes,
                        "stale": node.last_telemetry.is_some_and(
                            |t| t.elapsed() > Duration::from_secs(120),
                        ),
                    },
                    "providers": node.providers.iter().map(|p| {
                        serde_json::json!({
                            "provider": p.provider,
                            "models": p.models,
                        })
                    }).collect::<Vec<_>>(),
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
                    .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;
                Ok(serde_json::json!({}))
            })
        }),
    );

    // nodes.set_session: assign a node to a chat session
    reg.register(
        "nodes.set_session",
        Box::new(|ctx| {
            Box::pin(async move {
                let session_key = ctx
                    .params
                    .get("session_key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            "missing 'session_key' parameter",
                        )
                    })?;
                // node_id can be null to clear the node assignment.
                let node_id = ctx.params.get("node_id").and_then(|v| v.as_str());
                let resolved_node_id = if let Some(nid) = node_id {
                    if let Some(store) = ctx.state.credential_store.as_ref()
                        && let Some(target) = store
                            .resolve_ssh_target(nid)
                            .await
                            .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?
                    {
                        Some(target.node_id)
                    } else if let Some(target) = configured_legacy_ssh_target()
                        && crate::node_exec::ssh_target_matches(nid, &target)
                    {
                        Some(crate::node_exec::ssh_node_id(&target))
                    } else {
                        let inner = ctx.state.inner.read().await;
                        if inner.nodes.get(nid).is_none() {
                            return Err(ErrorShape::new(
                                error_codes::INVALID_REQUEST,
                                format!("node '{nid}' not found or not connected"),
                            ));
                        }
                        Some(nid.to_string())
                    }
                } else {
                    None
                };

                let Some(ref meta) = ctx.state.services.session_metadata else {
                    return Err(ErrorShape::new(
                        error_codes::UNAVAILABLE,
                        "session metadata not available",
                    ));
                };
                meta.upsert(session_key, None)
                    .await
                    .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;
                meta.set_node_id(session_key, resolved_node_id.as_deref())
                    .await
                    .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;
                Ok(serde_json::json!({ "ok": true, "node_id": resolved_node_id }))
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

                // Intercept telemetry events to cache data in NodeSession.
                if event == "node.telemetry"
                    && let Some(node_id) = payload.get("nodeId").and_then(|v| v.as_str())
                {
                    let mem_total = payload
                        .get("mem")
                        .and_then(|m| m.get("total"))
                        .and_then(|v| v.as_u64());
                    let mem_available = payload
                        .get("mem")
                        .and_then(|m| m.get("available"))
                        .and_then(|v| v.as_u64());
                    let cpu_count = payload
                        .get("cpuCount")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32);
                    let cpu_usage = payload
                        .get("cpuUsage")
                        .and_then(|v| v.as_f64())
                        .map(|v| v as f32);
                    let uptime_secs = payload.get("uptime").and_then(|v| v.as_u64());
                    let services: Vec<String> = payload
                        .get("services")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let disk_total = payload
                        .get("disk")
                        .and_then(|d| d.get("total"))
                        .and_then(|v| v.as_u64());
                    let disk_available = payload
                        .get("disk")
                        .and_then(|d| d.get("available"))
                        .and_then(|v| v.as_u64());
                    let runtimes: Vec<String> = payload
                        .get("runtimes")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();

                    let mut inner = ctx.state.inner.write().await;
                    let _ = inner.nodes.update_telemetry(
                        node_id,
                        mem_total,
                        mem_available,
                        cpu_count,
                        cpu_usage,
                        uptime_secs,
                        services,
                        disk_total,
                        disk_available,
                        runtimes,
                    );
                }

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

                        let write_mode = moltis_config::discover_and_load()
                            .memory
                            .user_profile_write_mode;
                        if write_mode.allows_auto_write() {
                            let mut user = moltis_config::resolve_user_profile();
                            user.location = Some(geo);
                            if let Err(e) = moltis_config::save_user_with_mode(&user, write_mode) {
                                tracing::warn!(error = %e, "failed to persist location to USER.md");
                            }
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
