use std::net::SocketAddr;

use {
    axum::{
        Json,
        extract::{ConnectInfo, Query, State, WebSocketUpgrade},
        http::StatusCode,
        response::IntoResponse,
    },
    moltis_httpd::AppState,
    tracing::warn,
};

use super::{
    auth::{is_local_connection, is_same_origin, websocket_header_authenticated},
    tmux::{
        host_terminal_apply_tmux_profile, host_terminal_ensure_tmux_session,
        host_terminal_normalize_window_name, host_terminal_tmux_available,
        host_terminal_tmux_create_window, host_terminal_tmux_list_windows,
    },
    types::{
        HOST_TERMINAL_SESSION_NAME, HostTerminalCreateWindowRequest, HostTerminalWindowInfo,
        HostTerminalWsQuery, TERMINAL_DISABLED, TERMINAL_SESSION_INIT_FAILED,
        TERMINAL_TMUX_UNAVAILABLE, TERMINAL_WINDOW_CREATE_FAILED, TERMINAL_WINDOW_NAME_INVALID,
        TERMINAL_WINDOWS_LIST_FAILED, terminal_error,
    },
    websocket::handle_terminal_ws_connection,
};

// ── Payload builders ─────────────────────────────────────────────────────────

fn host_terminal_windows_payload(
    windows: Vec<HostTerminalWindowInfo>,
    session_name: Option<&str>,
) -> serde_json::Value {
    let active_window_id = windows
        .iter()
        .find(|window| window.active)
        .map(|window| window.id.clone());
    serde_json::json!({
        "ok": true,
        "available": true,
        "sessionName": session_name,
        "windows": windows,
        "activeWindowId": active_window_id,
    })
}

// ── HTTP handlers ────────────────────────────────────────────────────────────

pub async fn api_terminal_windows_handler(State(state): State<AppState>) -> impl IntoResponse {
    if !state.gateway.config.server.is_terminal_enabled() {
        return (
            StatusCode::FORBIDDEN,
            Json(terminal_error(
                TERMINAL_DISABLED,
                "terminal has been disabled by the server administrator",
            )),
        )
            .into_response();
    }
    if !host_terminal_tmux_available() {
        return Json(serde_json::json!({
            "ok": true,
            "available": false,
            "sessionName": Option::<&str>::None,
            "windows": Vec::<HostTerminalWindowInfo>::new(),
            "activeWindowId": Option::<String>::None,
        }))
        .into_response();
    }
    if let Err(err) = host_terminal_ensure_tmux_session() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(terminal_error(
                TERMINAL_SESSION_INIT_FAILED,
                err.to_string(),
            )),
        )
            .into_response();
    }
    host_terminal_apply_tmux_profile();
    match host_terminal_tmux_list_windows() {
        Ok(windows) => Json(host_terminal_windows_payload(
            windows,
            Some(HOST_TERMINAL_SESSION_NAME),
        ))
        .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(terminal_error(
                TERMINAL_WINDOWS_LIST_FAILED,
                err.to_string(),
            )),
        )
            .into_response(),
    }
}

pub async fn api_terminal_windows_create_handler(
    State(state): State<AppState>,
    Json(payload): Json<HostTerminalCreateWindowRequest>,
) -> impl IntoResponse {
    if !state.gateway.config.server.is_terminal_enabled() {
        return (
            StatusCode::FORBIDDEN,
            Json(terminal_error(
                TERMINAL_DISABLED,
                "terminal has been disabled by the server administrator",
            )),
        )
            .into_response();
    }
    if !host_terminal_tmux_available() {
        return (
            StatusCode::CONFLICT,
            Json(terminal_error(
                TERMINAL_TMUX_UNAVAILABLE,
                "tmux is not available on host terminal",
            )),
        )
            .into_response();
    }
    let window_name = match payload
        .name
        .as_deref()
        .map(host_terminal_normalize_window_name)
        .transpose()
    {
        Ok(name) => name,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(terminal_error(
                    TERMINAL_WINDOW_NAME_INVALID,
                    err.to_string(),
                )),
            )
                .into_response();
        },
    };
    if let Err(err) = host_terminal_ensure_tmux_session() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(terminal_error(
                TERMINAL_SESSION_INIT_FAILED,
                err.to_string(),
            )),
        )
            .into_response();
    }
    match host_terminal_tmux_create_window(window_name.as_deref()) {
        Ok(window_id) => match host_terminal_tmux_list_windows() {
            Ok(windows) => {
                let created = windows
                    .iter()
                    .find(|window| window.id == window_id)
                    .cloned();
                Json(serde_json::json!({
                    "ok": true,
                    "window": created,
                    "windowId": window_id,
                    "sessionName": HOST_TERMINAL_SESSION_NAME,
                    "windows": windows,
                }))
                .into_response()
            },
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(terminal_error(
                    TERMINAL_WINDOWS_LIST_FAILED,
                    err.to_string(),
                )),
            )
                .into_response(),
        },
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(terminal_error(
                TERMINAL_WINDOW_CREATE_FAILED,
                err.to_string(),
            )),
        )
            .into_response(),
    }
}

/// Dedicated host terminal WebSocket stream (`Settings > Terminal`).
pub async fn api_terminal_ws_upgrade_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<HostTerminalWsQuery>,
    headers: axum::http::HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if !state.gateway.config.server.is_terminal_enabled() {
        return (
            StatusCode::FORBIDDEN,
            "terminal has been disabled by the server administrator",
        )
            .into_response();
    }

    // CSWSH protection: only same-origin browser upgrades are allowed.
    if let Some(origin) = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        let host = headers
            .get(axum::http::header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !is_same_origin(origin, host) {
            warn!(
                origin,
                host,
                remote = %addr,
                "rejected cross-origin terminal WebSocket upgrade"
            );
            return (
                StatusCode::FORBIDDEN,
                "cross-origin WebSocket connections are not allowed",
            )
                .into_response();
        }
    }

    let is_local = is_local_connection(&headers, addr, state.gateway.behind_proxy);
    let header_authenticated =
        websocket_header_authenticated(&headers, state.gateway.credential_store.as_ref(), is_local)
            .await;
    if !header_authenticated {
        return (
            StatusCode::UNAUTHORIZED,
            Json(terminal_error(
                "AUTH_NOT_AUTHENTICATED",
                "not authenticated",
            )),
        )
            .into_response();
    }

    let requested_window = query.window;
    ws.on_upgrade(move |socket| handle_terminal_ws_connection(socket, addr, requested_window))
        .into_response()
}
