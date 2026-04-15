use std::net::SocketAddr;

use {
    axum::extract::ws::{Message, WebSocket},
    base64::Engine as _,
    futures::{SinkExt, StreamExt},
    tracing::info,
};

use super::{
    pty::{
        host_terminal_resize, host_terminal_stop_runtime, host_terminal_write_input,
        spawn_host_terminal_runtime,
    },
    tmux::{
        host_terminal_apply_tmux_profile, host_terminal_default_window_target,
        host_terminal_ensure_tmux_session, host_terminal_resolve_window_target,
        host_terminal_tmux_available, host_terminal_tmux_install_hint,
        host_terminal_tmux_list_windows, host_terminal_tmux_reset_window_size,
        host_terminal_tmux_select_window,
    },
    types::{
        HOST_TERMINAL_DEFAULT_COLS, HOST_TERMINAL_DEFAULT_ROWS, HOST_TERMINAL_MAX_INPUT_BYTES,
        HOST_TERMINAL_SESSION_NAME, HostTerminalOutputEvent, HostTerminalWsClientMessage,
        HostTerminalWsControlAction, detect_host_root_user_for_terminal, host_terminal_user_name,
    },
};

// ── WebSocket helpers ────────────────────────────────────────────────────────

async fn terminal_ws_send_json(
    ws_tx: &mut futures::stream::SplitSink<WebSocket, Message>,
    payload: serde_json::Value,
) -> bool {
    match serde_json::to_string(&payload) {
        Ok(text) => ws_tx.send(Message::Text(text.into())).await.is_ok(),
        Err(err) => {
            tracing::warn!(error = %err, "failed to serialize terminal ws payload");
            false
        },
    }
}

async fn terminal_ws_send_status(
    ws_tx: &mut futures::stream::SplitSink<WebSocket, Message>,
    text: impl std::fmt::Display,
    level: &str,
) -> bool {
    terminal_ws_send_json(
        ws_tx,
        serde_json::json!({
            "type": "status",
            "text": text.to_string(),
            "level": level,
        }),
    )
    .await
}

async fn terminal_ws_send_output(
    ws_tx: &mut futures::stream::SplitSink<WebSocket, Message>,
    data: &[u8],
) -> bool {
    let encoded = base64::engine::general_purpose::STANDARD.encode(data);
    terminal_ws_send_json(
        ws_tx,
        serde_json::json!({
            "type": "output",
            "encoding": "base64",
            "data": encoded,
        }),
    )
    .await
}

// ── WebSocket connection handler ─────────────────────────────────────────────

pub(crate) async fn handle_terminal_ws_connection(
    socket: WebSocket,
    remote_addr: SocketAddr,
    requested_window: Option<String>,
) {
    let conn_id = uuid::Uuid::new_v4().to_string();
    info!(conn_id = %conn_id, remote = %remote_addr, "terminal ws: new connection");

    let (mut ws_tx, mut ws_rx) = socket.split();

    let is_root = detect_host_root_user_for_terminal();
    let prompt_symbol = if is_root.unwrap_or(false) {
        "#"
    } else {
        "$"
    };
    let user = host_terminal_user_name();
    let persistence_available = host_terminal_tmux_available();
    let tmux_install_command = host_terminal_tmux_install_hint();
    let mut current_window_target: Option<String> = None;
    if persistence_available {
        if let Err(err) = host_terminal_ensure_tmux_session() {
            let _ = terminal_ws_send_status(&mut ws_tx, &err, "error").await;
            return;
        }
        host_terminal_apply_tmux_profile();
        let windows = match host_terminal_tmux_list_windows() {
            Ok(windows) => windows,
            Err(err) => {
                let _ = terminal_ws_send_status(&mut ws_tx, &err, "error").await;
                return;
            },
        };
        let fallback_window_target = host_terminal_default_window_target(&windows);
        if let Some(requested) = requested_window.as_deref() {
            match host_terminal_resolve_window_target(&windows, requested) {
                Some(target) => {
                    current_window_target = Some(target);
                },
                None => {
                    if let Some(fallback) = fallback_window_target {
                        current_window_target = Some(fallback);
                        let _ = terminal_ws_send_status(
                            &mut ws_tx,
                            "requested terminal window no longer exists, attached to the current window",
                            "info",
                        )
                        .await;
                    } else {
                        let _ = terminal_ws_send_status(
                            &mut ws_tx,
                            "requested terminal window does not exist",
                            "error",
                        )
                        .await;
                        return;
                    }
                },
            }
        } else {
            current_window_target = fallback_window_target;
        }
    }
    let mut current_cols = HOST_TERMINAL_DEFAULT_COLS;
    let mut current_rows = HOST_TERMINAL_DEFAULT_ROWS;
    let mut runtime = match spawn_host_terminal_runtime(
        current_cols,
        current_rows,
        persistence_available,
        current_window_target.as_deref(),
    ) {
        Ok(runtime) => runtime,
        Err(err) => {
            let _ = terminal_ws_send_status(&mut ws_tx, &err, "error").await;
            return;
        },
    };

    if !terminal_ws_send_json(
        &mut ws_tx,
        serde_json::json!({
            "type": "ready",
            "available": true,
            "mode": "host",
            "sandboxed": false,
            "user": user,
            "isRoot": is_root,
            "promptSymbol": prompt_symbol,
            "persistenceAvailable": persistence_available,
            "persistenceEnabled": persistence_available,
            "persistenceMode": if persistence_available { "tmux" } else { "ephemeral" },
            "sessionName": if persistence_available { Some(HOST_TERMINAL_SESSION_NAME) } else { None::<&str> },
            "activeWindowId": current_window_target.clone(),
            "tmuxInstallCommand": tmux_install_command,
        }),
    )
    .await
    {
        host_terminal_stop_runtime(&mut runtime);
        return;
    }

    if !persistence_available && let Some(install_cmd) = host_terminal_tmux_install_hint() {
        let hint = format!(
            "tmux is not installed, session persistence is disabled. Install tmux for persistence: {install_cmd}"
        );
        if !terminal_ws_send_status(&mut ws_tx, &hint, "info").await {
            host_terminal_stop_runtime(&mut runtime);
            return;
        }
    }

    loop {
        tokio::select! {
            maybe_output = runtime.output_rx.recv() => {
                match maybe_output {
                    Some(HostTerminalOutputEvent::Output(data)) => {
                        if !terminal_ws_send_output(&mut ws_tx, &data).await {
                            break;
                        }
                    }
                    Some(HostTerminalOutputEvent::Error(err)) => {
                        if !terminal_ws_send_status(&mut ws_tx, &err, "error").await {
                            break;
                        }
                    }
                    Some(HostTerminalOutputEvent::Closed) | None => {
                        let _ = terminal_ws_send_status(
                            &mut ws_tx,
                            "host terminal process exited",
                            "error",
                        )
                        .await;
                        break;
                    }
                }
            }
            maybe_msg = ws_rx.next() => {
                let Some(msg_result) = maybe_msg else {
                    break;
                };
                let Ok(msg) = msg_result else {
                    break;
                };

                match msg {
                    Message::Text(text) => {
                        if text.len() > HOST_TERMINAL_MAX_INPUT_BYTES * 2 {
                            if !terminal_ws_send_status(
                                &mut ws_tx,
                                "terminal ws message too large",
                                "error",
                            )
                            .await
                            {
                                break;
                            }
                            continue;
                        }

                        let parsed: Result<HostTerminalWsClientMessage, _> = serde_json::from_str(&text);
                        match parsed {
                            Ok(HostTerminalWsClientMessage::Input { data }) => {
                                if data.is_empty() {
                                    continue;
                                }
                                if data.len() > HOST_TERMINAL_MAX_INPUT_BYTES {
                                    if !terminal_ws_send_status(
                                        &mut ws_tx,
                                        &format!(
                                            "input chunk too large (max {} bytes)",
                                            HOST_TERMINAL_MAX_INPUT_BYTES
                                        ),
                                        "error",
                                    )
                                    .await
                                    {
                                        break;
                                    }
                                    continue;
                                }
                                if let Err(err) = host_terminal_write_input(&mut runtime, &data) {
                                    if !terminal_ws_send_status(&mut ws_tx, &err, "error").await {
                                        break;
                                    }
                                    continue;
                                }
                            }
                            Ok(HostTerminalWsClientMessage::Resize {
                                cols: next_cols,
                                rows: next_rows,
                            }) => {
                                if next_cols < 2 || next_rows < 1 {
                                    continue;
                                }
                                if let Err(err) = host_terminal_resize(&runtime, next_cols, next_rows) {
                                    if !terminal_ws_send_status(&mut ws_tx, &err, "error").await {
                                        break;
                                    }
                                } else {
                                    // Keep restart size aligned with latest client viewport.
                                    current_cols = next_cols;
                                    current_rows = next_rows;
                                    // Force tmux to recalculate window dimensions after
                                    // the PTY resize so the window matches the client
                                    // viewport (tmux may not react to SIGWINCH alone).
                                    if persistence_available {
                                        host_terminal_tmux_reset_window_size(
                                            current_window_target.as_deref(),
                                        );
                                    }
                                }
                            }
                            Ok(HostTerminalWsClientMessage::SwitchWindow { window }) => {
                                if !persistence_available {
                                    if !terminal_ws_send_status(
                                        &mut ws_tx,
                                        "tmux window switching is unavailable",
                                        "error",
                                    )
                                    .await
                                    {
                                        break;
                                    }
                                    continue;
                                }
                                let windows = match host_terminal_tmux_list_windows() {
                                    Ok(windows) => windows,
                                    Err(err) => {
                                        if !terminal_ws_send_status(&mut ws_tx, &err, "error").await {
                                            break;
                                        }
                                        continue;
                                    }
                                };
                                let Some(target_window_id) =
                                    host_terminal_resolve_window_target(&windows, &window)
                                else {
                                    if !terminal_ws_send_status(
                                        &mut ws_tx,
                                        "requested terminal window does not exist",
                                        "error",
                                    )
                                    .await
                                    {
                                        break;
                                    }
                                    continue;
                                };
                                if let Err(err) = host_terminal_tmux_select_window(&target_window_id) {
                                    if !terminal_ws_send_status(&mut ws_tx, &err, "error").await {
                                        break;
                                    }
                                    continue;
                                }
                                host_terminal_tmux_reset_window_size(Some(&target_window_id));
                                if let Err(err) = host_terminal_resize(&runtime, current_cols, current_rows) {
                                    if !terminal_ws_send_status(&mut ws_tx, &err, "error").await {
                                        break;
                                    }
                                    continue;
                                }
                                current_window_target = Some(target_window_id.clone());
                                if !terminal_ws_send_json(
                                    &mut ws_tx,
                                    serde_json::json!({
                                        "type": "active_window",
                                        "windowId": target_window_id,
                                    }),
                                )
                                .await
                                {
                                    break;
                                }
                            }
                            Ok(HostTerminalWsClientMessage::Control { action }) => {
                                let action_result = match action {
                                    HostTerminalWsControlAction::Restart => {
                                        host_terminal_stop_runtime(&mut runtime);
                                        match spawn_host_terminal_runtime(
                                            current_cols,
                                            current_rows,
                                            persistence_available,
                                            current_window_target.as_deref(),
                                        ) {
                                            Ok(next_runtime) => {
                                                runtime = next_runtime;
                                                Ok(())
                                            }
                                            Err(err) => Err(err),
                                        }
                                    }
                                    HostTerminalWsControlAction::CtrlC => {
                                        host_terminal_write_input(&mut runtime, "\u{3}")
                                    }
                                    HostTerminalWsControlAction::Clear => {
                                        host_terminal_write_input(&mut runtime, "\u{c}")
                                    }
                                };
                                if let Err(err) = action_result
                                    && !terminal_ws_send_status(&mut ws_tx, &err, "error").await
                                {
                                    break;
                                }
                            }
                            Ok(HostTerminalWsClientMessage::Ping) => {
                                if !terminal_ws_send_json(
                                    &mut ws_tx,
                                    serde_json::json!({ "type": "pong" }),
                                )
                                .await
                                {
                                    break;
                                }
                            }
                            Err(err) => {
                                if !terminal_ws_send_status(
                                    &mut ws_tx,
                                    &format!("invalid terminal ws message: {err}"),
                                    "error",
                                )
                                .await
                                {
                                    break;
                                }
                            }
                        }
                    }
                    Message::Ping(payload) => {
                        if ws_tx.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    Message::Binary(_) | Message::Pong(_) => {}
                }
            }
        }
    }

    host_terminal_stop_runtime(&mut runtime);
    info!(conn_id = %conn_id, remote = %remote_addr, "terminal ws: connection closed");
}
