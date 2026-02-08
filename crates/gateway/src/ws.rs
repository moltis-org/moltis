use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use {
    axum::extract::ws::{Message, WebSocket},
    futures::{SinkExt, stream::StreamExt},
    tokio::sync::mpsc,
    tracing::{debug, info, warn},
};

use moltis_protocol::{
    ConnectParams, ErrorShape, EventFrame, Features, GatewayFrame, HANDSHAKE_TIMEOUT_MS, HelloAuth,
    HelloOk, MAX_PAYLOAD_BYTES, PROTOCOL_VERSION, Policy, ResponseFrame, ServerInfo, error_codes,
};

use crate::{
    auth,
    broadcast::{BroadcastOpts, broadcast},
    methods::{MethodContext, MethodRegistry},
    nodes::NodeSession,
    state::{ConnectedClient, GatewayState},
};

fn top_level_param_keys(params: &Option<serde_json::Value>) -> Vec<String> {
    params
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default()
}

/// Handle a single WebSocket connection through its full lifecycle:
/// handshake (with auth) → message loop → cleanup.
pub async fn handle_connection(
    socket: WebSocket,
    state: Arc<GatewayState>,
    methods: Arc<MethodRegistry>,
    remote_addr: SocketAddr,
    accept_language: Option<String>,
    header_authenticated: bool,
) {
    let conn_id = uuid::Uuid::new_v4().to_string();
    let remote_ip = remote_addr.ip().to_string();
    info!(conn_id = %conn_id, remote_ip = %remote_ip, "ws: new connection");

    let (mut ws_tx, mut ws_rx) = socket.split();
    let (client_tx, mut client_rx) = mpsc::unbounded_channel::<String>();

    // Spawn write loop: forwards frames from the client_tx channel to the WebSocket.
    let write_conn_id = conn_id.clone();
    let write_handle = tokio::spawn(async move {
        while let Some(msg) = client_rx.recv().await {
            if ws_tx.send(Message::Text(msg.into())).await.is_err() {
                debug!(conn_id = %write_conn_id, "ws: write loop closed");
                break;
            }
        }
    });

    // ── Handshake phase ──────────────────────────────────────────────────

    let connect_result = match tokio::time::timeout(
        std::time::Duration::from_millis(HANDSHAKE_TIMEOUT_MS),
        wait_for_connect(&mut ws_rx),
    )
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            warn!(conn_id = %conn_id, error = %e, "ws: handshake failed");
            drop(client_tx);
            write_handle.abort();
            return;
        },
        Err(_) => {
            warn!(conn_id = %conn_id, "ws: handshake timeout");
            drop(client_tx);
            write_handle.abort();
            return;
        },
    };

    let (request_id, params) = connect_result;

    if state.ws_request_logs {
        let connect_param_keys = serde_json::to_value(&params)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .map(|obj| obj.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        info!(
            conn_id = %conn_id,
            request_id = %request_id,
            method = "connect",
            param_keys = ?connect_param_keys,
            "ws: received request frame"
        );
    }

    // Validate protocol version.
    if params.min_protocol > PROTOCOL_VERSION || params.max_protocol < PROTOCOL_VERSION {
        let err = ResponseFrame::err(
            &request_id,
            ErrorShape::new(
                error_codes::INVALID_REQUEST,
                format!(
                    "protocol mismatch: server={}, client={}-{}",
                    PROTOCOL_VERSION, params.min_protocol, params.max_protocol
                ),
            ),
        );
        let _ = client_tx.send(serde_json::to_string(&err).unwrap());
        drop(client_tx);
        write_handle.abort();
        return;
    }

    // ── Auth validation ──────────────────────────────────────────────────
    let is_loopback = auth::is_loopback(&remote_ip);

    // Try credential-store auth first (API key, password hash), then fall
    // back to legacy env-var auth.
    let mut authenticated = is_loopback || header_authenticated;
    // Scopes from API key verification (if any).
    let mut api_key_scopes: Option<Vec<String>> = None;

    if !authenticated && let Some(ref cred_store) = state.credential_store {
        if cred_store.is_setup_complete() {
            // Check API key.
            if let Some(ref api_key) = params.auth.as_ref().and_then(|a| a.api_key.clone())
                && let Ok(Some(verification)) = cred_store.verify_api_key(api_key).await
            {
                authenticated = true;
                // Store the scopes from the API key (empty = full access)
                api_key_scopes = Some(verification.scopes);
            }
            // Check password against DB hash.
            if !authenticated
                && let Some(ref pw) = params.auth.as_ref().and_then(|a| a.password.clone())
                && cred_store.verify_password(pw).await.unwrap_or(false)
            {
                authenticated = true;
            }
        } else {
            // Setup not complete yet — allow all connections.
            authenticated = true;
        }
    }

    // Fall back to legacy env-var auth if credential store didn't authenticate.
    if !authenticated {
        let has_legacy_auth = state.auth.token.is_some() || state.auth.password.is_some();
        if has_legacy_auth {
            let provided_token = params.auth.as_ref().and_then(|a| a.token.as_deref());
            let provided_password = params.auth.as_ref().and_then(|a| a.password.as_deref());
            let auth_result = auth::authorize_connect(
                &state.auth,
                provided_token,
                provided_password,
                Some(&remote_ip),
            );
            if auth_result.ok {
                authenticated = true;
            }
        } else if state.credential_store.is_none() {
            // No auth configured at all — grant access (backward compat).
            authenticated = true;
        }
    }

    if !authenticated {
        warn!(conn_id = %conn_id, "ws: auth failed");
        let err = ResponseFrame::err(
            &request_id,
            ErrorShape::new(error_codes::INVALID_REQUEST, "authentication failed"),
        );
        let _ = client_tx.send(serde_json::to_string(&err).unwrap());
        drop(client_tx);
        write_handle.abort();
        return;
    }

    let role = params.role.clone().unwrap_or_else(|| "operator".into());

    // Determine scopes: use API key scopes if provided, otherwise default to full access.
    // Empty API key scopes means full access (backward compatibility).
    let scopes = match api_key_scopes {
        Some(key_scopes) if !key_scopes.is_empty() => key_scopes,
        _ => {
            // Full access: either no API key used, or API key has no scope restrictions
            vec![
                "operator.admin".into(),
                "operator.read".into(),
                "operator.write".into(),
                "operator.approvals".into(),
                "operator.pairing".into(),
            ]
        },
    };

    // Build HelloOk with auth info.
    let hello_auth = HelloAuth {
        device_token: String::new(),
        role: role.clone(),
        scopes: scopes.clone(),
        issued_at_ms: Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        ),
    };

    let hello = HelloOk {
        r#type: "hello-ok".into(),
        protocol: PROTOCOL_VERSION,
        server: ServerInfo {
            version: state.version.clone(),
            commit: None,
            host: Some(state.hostname.clone()),
            conn_id: conn_id.clone(),
        },
        features: Features {
            methods: methods.method_names(),
            events: vec![
                "tick".into(),
                "shutdown".into(),
                "agent".into(),
                "chat".into(),
                "presence".into(),
                "health".into(),
                "exec.approval.requested".into(),
                "exec.approval.resolved".into(),
                "device.pair.requested".into(),
                "device.pair.resolved".into(),
                "node.pair.requested".into(),
                "node.pair.resolved".into(),
                "node.invoke.request".into(),
            ],
        },
        snapshot: serde_json::json!({}),
        canvas_host_url: None,
        auth: Some(hello_auth),
        policy: Policy::default_policy(),
    };
    let resp = ResponseFrame::ok(&request_id, serde_json::to_value(&hello).unwrap());
    let _ = client_tx.send(serde_json::to_string(&resp).unwrap());

    info!(
        conn_id = %conn_id,
        client_id = %params.client.id,
        client_version = %params.client.version,
        role = %role,
        "ws: handshake complete"
    );

    // Register the client with server-resolved scopes so broadcast guards work.
    let now = std::time::Instant::now();
    let mut resolved_params = params.clone();
    resolved_params.scopes = Some(scopes.clone());
    resolved_params.role = Some(role.clone());
    let client = ConnectedClient {
        conn_id: conn_id.clone(),
        connect_params: resolved_params,
        sender: client_tx.clone(),
        connected_at: now,
        last_activity: now,
        accept_language,
    };
    state.register_client(client).await;

    // If node role, register in node registry.
    if role == "node" {
        let caps = params.caps.clone().unwrap_or_default();
        let commands = params.commands.clone().unwrap_or_default();
        let permissions: HashMap<String, bool> = params
            .permissions
            .as_ref()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_bool().map(|b| (k.clone(), b)))
                    .collect()
            })
            .unwrap_or_default();

        let node = NodeSession {
            node_id: params.client.id.clone(),
            conn_id: conn_id.clone(),
            display_name: params.client.display_name.clone(),
            platform: params.client.platform.clone(),
            version: params.client.version.clone(),
            capabilities: caps,
            commands,
            permissions,
            path_env: params.path_env.clone(),
            remote_ip: Some(remote_ip.clone()),
            connected_at: now,
        };
        state.nodes.write().await.register(node);
        info!(conn_id = %conn_id, node_id = %params.client.id, "node registered");

        // Broadcast presence change.
        broadcast(
            &state,
            "presence",
            serde_json::json!({
                "type": "node.connected",
                "nodeId": params.client.id,
                "platform": params.client.platform,
            }),
            BroadcastOpts::default(),
        )
        .await;
    }

    // ── Message loop ─────────────────────────────────────────────────────

    while let Some(msg) = ws_rx.next().await {
        let text = match msg {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Close(_)) => break,
            Ok(_) => continue,
            Err(e) => {
                debug!(conn_id = %conn_id, error = %e, "ws: read error");
                break;
            },
        };

        // Enforce payload size limit.
        if text.len() > MAX_PAYLOAD_BYTES {
            warn!(conn_id = %conn_id, size = text.len(), "ws: payload too large");
            let err = EventFrame::new(
                "error",
                serde_json::json!({ "message": "payload too large", "maxBytes": MAX_PAYLOAD_BYTES }),
                state.next_seq(),
            );
            let _ = client_tx.send(serde_json::to_string(&err).unwrap());
            continue;
        }

        let frame: GatewayFrame = match serde_json::from_str(&text) {
            Ok(f) => f,
            Err(e) => {
                warn!(conn_id = %conn_id, error = %e, "ws: invalid frame");
                let err = EventFrame::new(
                    "error",
                    serde_json::json!({ "message": "invalid frame" }),
                    state.next_seq(),
                );
                let _ = client_tx.send(serde_json::to_string(&err).unwrap());
                continue;
            },
        };

        // Touch activity timestamp.
        if let Some(client) = state.clients.write().await.get_mut(&conn_id) {
            client.touch();
        }

        match frame {
            GatewayFrame::Request(req) => {
                if state.ws_request_logs {
                    info!(
                        conn_id = %conn_id,
                        request_id = %req.id,
                        method = %req.method,
                        param_keys = ?top_level_param_keys(&req.params),
                        "ws: received request frame"
                    );
                }
                let ctx = MethodContext {
                    request_id: req.id.clone(),
                    method: req.method.clone(),
                    params: req.params.unwrap_or(serde_json::Value::Null),
                    client_conn_id: conn_id.clone(),
                    client_role: role.clone(),
                    client_scopes: scopes.clone(),
                    state: Arc::clone(&state),
                };
                let response = methods.dispatch(ctx).await;
                if state.ws_request_logs {
                    info!(
                        conn_id = %conn_id,
                        request_id = %req.id,
                        method = %req.method,
                        ok = response.ok,
                        "ws: sent response frame"
                    );
                }
                let _ = client_tx.send(serde_json::to_string(&response).unwrap());
            },
            _ => {
                debug!(conn_id = %conn_id, "ws: ignoring non-request frame");
            },
        }
    }

    // ── Cleanup ──────────────────────────────────────────────────────────

    // Unregister node if applicable.
    let removed_node = state.nodes.write().await.unregister_by_conn(&conn_id);
    if let Some(node) = &removed_node {
        info!(conn_id = %conn_id, node_id = %node.node_id, "node unregistered");
        broadcast(
            &state,
            "presence",
            serde_json::json!({
                "type": "node.disconnected",
                "nodeId": node.node_id,
            }),
            BroadcastOpts::default(),
        )
        .await;
    }

    let duration = state
        .remove_client(&conn_id)
        .await
        .map(|c| c.connected_at.elapsed())
        .unwrap_or_default();

    info!(
        conn_id = %conn_id,
        duration_secs = duration.as_secs(),
        "ws: connection closed"
    );

    drop(client_tx);
    write_handle.abort();
}

/// Wait for the first `connect` request frame.
async fn wait_for_connect(
    rx: &mut futures::stream::SplitStream<WebSocket>,
) -> anyhow::Result<(String, ConnectParams)> {
    while let Some(msg) = rx.next().await {
        let text = match msg? {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => anyhow::bail!("connection closed before handshake"),
            _ => continue,
        };

        let frame: GatewayFrame = serde_json::from_str(&text)?;
        match frame {
            GatewayFrame::Request(req) => {
                if req.method != "connect" {
                    anyhow::bail!("first message must be 'connect', got '{}'", req.method);
                }
                let params: ConnectParams =
                    serde_json::from_value(req.params.unwrap_or(serde_json::Value::Null))?;
                return Ok((req.id, params));
            },
            _ => anyhow::bail!("first message must be a request frame"),
        }
    }
    anyhow::bail!("connection closed before handshake")
}
