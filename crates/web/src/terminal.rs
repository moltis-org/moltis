use std::{
    io::{Read, Write},
    net::SocketAddr,
    path::PathBuf,
    process::Command,
    sync::Arc,
};

use {
    axum::{
        Json,
        extract::{
            ConnectInfo, Query, State, WebSocketUpgrade,
            ws::{Message, WebSocket},
        },
        http::StatusCode,
        response::IntoResponse,
    },
    base64::Engine as _,
    futures::{SinkExt, StreamExt},
    moltis_gateway::server::AppState,
    portable_pty::{CommandBuilder, PtySize, native_pty_system},
    tracing::{debug, info, warn},
};

// ── Constants ────────────────────────────────────────────────────────────────

const HOST_TERMINAL_SESSION_NAME: &str = "moltis-host-terminal";
const HOST_TERMINAL_TMUX_SOCKET_NAME: &str = "moltis-host-terminal";
const HOST_TERMINAL_TMUX_CONFIG_PATH: &str = "/dev/null";
const HOST_TERMINAL_MAX_INPUT_BYTES: usize = 8 * 1024;
const HOST_TERMINAL_DEFAULT_COLS: u16 = 220;
const HOST_TERMINAL_DEFAULT_ROWS: u16 = 56;
const TERMINAL_SESSION_INIT_FAILED: &str = "TERMINAL_SESSION_INIT_FAILED";
const TERMINAL_WINDOWS_LIST_FAILED: &str = "TERMINAL_WINDOWS_LIST_FAILED";
const TERMINAL_TMUX_UNAVAILABLE: &str = "TERMINAL_TMUX_UNAVAILABLE";
const TERMINAL_WINDOW_NAME_INVALID: &str = "TERMINAL_WINDOW_NAME_INVALID";
const TERMINAL_WINDOW_CREATE_FAILED: &str = "TERMINAL_WINDOW_CREATE_FAILED";

// ── Data structures ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct HostTerminalWsQuery {
    window: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct HostTerminalWindowInfo {
    id: String,
    index: u32,
    name: String,
    active: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct HostTerminalCreateWindowRequest {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HostTerminalWsClientMessage {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
    SwitchWindow { window: String },
    Control { action: HostTerminalWsControlAction },
    Ping,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum HostTerminalWsControlAction {
    Restart,
    CtrlC,
    Clear,
}

enum HostTerminalOutputEvent {
    Output(Vec<u8>),
    Error(String),
    Closed,
}

struct HostTerminalPtyRuntime {
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    output_rx: tokio::sync::mpsc::UnboundedReceiver<HostTerminalOutputEvent>,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct TerminalError {
    message: String,
}

impl From<String> for TerminalError {
    fn from(message: String) -> Self {
        Self { message }
    }
}

impl From<&str> for TerminalError {
    fn from(message: &str) -> Self {
        Self {
            message: message.to_string(),
        }
    }
}

type TerminalResult<T> = Result<T, TerminalError>;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn terminal_error(code: &str, error: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "code": code,
        "error": error.into(),
    })
}

fn host_terminal_working_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
}

fn host_terminal_user_name() -> String {
    std::env::var("USER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("LOGNAME")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn host_terminal_tmux_available() -> bool {
    if cfg!(windows) {
        return false;
    }
    which::which("tmux").is_ok()
}

fn tmux_install_command_for_linux(
    has_debian: bool,
    has_redhat: bool,
    has_arch: bool,
    has_alpine: bool,
) -> &'static str {
    if has_debian {
        return "sudo apt install tmux";
    }
    if has_redhat {
        return "sudo dnf install tmux";
    }
    if has_arch {
        return "sudo pacman -S tmux";
    }
    if has_alpine {
        return "sudo apk add tmux";
    }
    "install tmux using your package manager"
}

fn tmux_install_command_for_host_os() -> Option<&'static str> {
    if cfg!(windows) {
        return None;
    }
    if cfg!(target_os = "macos") {
        return Some("brew install tmux");
    }
    if cfg!(target_os = "linux") {
        return Some(tmux_install_command_for_linux(
            std::path::Path::new("/etc/debian_version").exists(),
            std::path::Path::new("/etc/redhat-release").exists(),
            std::path::Path::new("/etc/arch-release").exists(),
            std::path::Path::new("/etc/alpine-release").exists(),
        ));
    }
    Some("install tmux using your package manager")
}

fn host_terminal_tmux_install_hint() -> Option<String> {
    tmux_install_command_for_host_os().map(str::to_string)
}

fn host_terminal_apply_env(cmd: &mut CommandBuilder) {
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("TMUX", "");
}

fn host_terminal_apply_tmux_common_args(cmd: &mut CommandBuilder) {
    cmd.args([
        "-L",
        HOST_TERMINAL_TMUX_SOCKET_NAME,
        "-f",
        HOST_TERMINAL_TMUX_CONFIG_PATH,
    ]);
}

fn host_terminal_tmux_command() -> Command {
    let mut cmd = Command::new("tmux");
    cmd.args([
        "-L",
        HOST_TERMINAL_TMUX_SOCKET_NAME,
        "-f",
        HOST_TERMINAL_TMUX_CONFIG_PATH,
    ]);
    cmd
}

fn host_terminal_apply_tmux_profile() {
    let commands: &[&[&str]] = &[
        &["set-option", "-g", "status", "off"],
        &["set-option", "-g", "mouse", "off"],
        &["set-window-option", "-g", "window-size", "latest"],
        &["set-option", "-g", "allow-rename", "off"],
        &["set-window-option", "-g", "automatic-rename", "off"],
        &["set-option", "-g", "set-titles", "off"],
        &["set-option", "-g", "renumber-windows", "on"],
    ];
    for args in commands {
        let mut cmd = host_terminal_tmux_command();
        cmd.args(*args);
        match cmd.output() {
            Ok(output) if output.status.success() => {},
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stderr = stderr.trim();
                if stderr.is_empty() {
                    debug!(
                        command = ?args,
                        status = %output.status,
                        "tmux profile command failed for host terminal"
                    );
                } else {
                    debug!(
                        command = ?args,
                        status = %output.status,
                        error = stderr,
                        "tmux profile command failed for host terminal"
                    );
                }
            },
            Err(err) => {
                debug!(
                    command = ?args,
                    error = %err,
                    "failed to execute tmux profile command for host terminal"
                );
            },
        }
    }
}

fn host_terminal_normalize_window_name(name: &str) -> TerminalResult<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("window name cannot be empty".into());
    }
    if trimmed.chars().count() > 64 {
        return Err("window name must be 64 characters or fewer".into());
    }
    Ok(trimmed.to_string())
}

fn host_terminal_normalize_window_target(target: &str) -> Option<String> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix('@') {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            return Some(trimmed.to_string());
        }
        return None;
    }
    if trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Some(trimmed.to_string());
    }
    None
}

fn host_terminal_resolve_window_target(
    windows: &[HostTerminalWindowInfo],
    requested: &str,
) -> Option<String> {
    let normalized = host_terminal_normalize_window_target(requested)?;
    if normalized.starts_with('@') {
        return windows
            .iter()
            .find(|window| window.id == normalized)
            .map(|window| window.id.clone());
    }
    let requested_index = normalized.parse::<u32>().ok()?;
    windows
        .iter()
        .find(|window| window.index == requested_index)
        .map(|window| window.id.clone())
}

fn host_terminal_default_window_target(windows: &[HostTerminalWindowInfo]) -> Option<String> {
    windows
        .iter()
        .find(|window| window.active)
        .or_else(|| windows.first())
        .map(|window| window.id.clone())
}

fn host_terminal_ensure_tmux_session() -> TerminalResult<()> {
    let mut has_cmd = host_terminal_tmux_command();
    let has_output = has_cmd
        .args(["has-session", "-t", HOST_TERMINAL_SESSION_NAME])
        .output()
        .map_err(|err| format!("failed to check tmux session: {err}"))?;
    if has_output.status.success() {
        return Ok(());
    }

    let mut create_cmd = host_terminal_tmux_command();
    create_cmd.args(["new-session", "-d", "-s", HOST_TERMINAL_SESSION_NAME]);
    if let Some(working_dir) = host_terminal_working_dir() {
        create_cmd.arg("-c").arg(working_dir);
    }
    let create_output = create_cmd
        .output()
        .map_err(|err| format!("failed to create tmux session: {err}"))?;
    if create_output.status.success() {
        return Ok(());
    }

    let mut retry_has_cmd = host_terminal_tmux_command();
    let retry_has_output = retry_has_cmd
        .args(["has-session", "-t", HOST_TERMINAL_SESSION_NAME])
        .output()
        .map_err(|err| format!("failed to re-check tmux session: {err}"))?;
    if retry_has_output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&create_output.stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        Err(format!(
            "failed to create tmux session '{}' (exit {})",
            HOST_TERMINAL_SESSION_NAME, create_output.status
        )
        .into())
    } else {
        Err(format!(
            "failed to create tmux session '{}': {}",
            HOST_TERMINAL_SESSION_NAME, stderr
        )
        .into())
    }
}

fn host_terminal_parse_tmux_window_line(line: &str) -> Option<HostTerminalWindowInfo> {
    let mut parts = line.splitn(4, '\t');
    let id = parts.next()?.trim();
    let index = parts.next()?.trim().parse::<u32>().ok()?;
    let name = parts.next()?.trim();
    let active_raw = parts.next()?.trim();
    let active = active_raw == "1";
    let id = host_terminal_normalize_window_target(id).filter(|value| value.starts_with('@'))?;
    Some(HostTerminalWindowInfo {
        id,
        index,
        name: name.to_string(),
        active,
    })
}

fn host_terminal_tmux_list_windows() -> TerminalResult<Vec<HostTerminalWindowInfo>> {
    let mut cmd = host_terminal_tmux_command();
    let output = cmd
        .args([
            "list-windows",
            "-t",
            HOST_TERMINAL_SESSION_NAME,
            "-F",
            "#{window_id}\t#{window_index}\t#{window_name}\t#{window_active}",
        ])
        .output()
        .map_err(|err| format!("failed to list tmux windows: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            return Err(format!("failed to list tmux windows (exit {})", output.status).into());
        }
        return Err(format!("failed to list tmux windows: {stderr}").into());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut windows: Vec<HostTerminalWindowInfo> = stdout
        .lines()
        .filter_map(host_terminal_parse_tmux_window_line)
        .collect();
    windows.sort_by_key(|window| window.index);
    Ok(windows)
}

fn host_terminal_tmux_create_window(name: Option<&str>) -> TerminalResult<String> {
    let mut cmd = host_terminal_tmux_command();
    cmd.args([
        "new-window",
        "-d",
        "-t",
        HOST_TERMINAL_SESSION_NAME,
        "-P",
        "-F",
        "#{window_id}",
    ]);
    if let Some(name) = name {
        cmd.args(["-n", name]);
    }
    let output = cmd
        .output()
        .map_err(|err| format!("failed to create tmux window: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            return Err(format!("failed to create tmux window (exit {})", output.status).into());
        }
        return Err(format!("failed to create tmux window: {stderr}").into());
    }
    let window_id_raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let window_id = host_terminal_normalize_window_target(&window_id_raw)
        .filter(|value| value.starts_with('@'))
        .ok_or_else(|| "tmux did not return a valid window id".to_string())?;
    Ok(window_id)
}

fn host_terminal_tmux_select_window(window_target: &str) -> TerminalResult<()> {
    let mut cmd = host_terminal_tmux_command();
    let output = cmd
        .args(["select-window", "-t", window_target])
        .output()
        .map_err(|err| format!("failed to select tmux window: {err}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        Err(format!(
            "failed to select tmux window '{}' (exit {})",
            window_target, output.status
        )
        .into())
    } else {
        Err(format!(
            "failed to select tmux window '{}': {}",
            window_target, stderr
        )
        .into())
    }
}

fn host_terminal_tmux_reset_window_size(window_target: Option<&str>) {
    let target = window_target.unwrap_or(HOST_TERMINAL_SESSION_NAME);
    let mut cmd = host_terminal_tmux_command();
    let output = cmd.args(["resize-window", "-A", "-t", target]).output();
    match output {
        Ok(output) if output.status.success() => {},
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr = stderr.trim();
            if stderr.is_empty() {
                debug!(
                    target,
                    status = %output.status,
                    "tmux resize-window -A failed while resetting host terminal window size"
                );
            } else {
                debug!(
                    target,
                    status = %output.status,
                    error = stderr,
                    "tmux resize-window -A failed while resetting host terminal window size"
                );
            }
        },
        Err(err) => {
            debug!(
                target,
                error = %err,
                "failed to invoke tmux resize-window -A for host terminal window size reset"
            );
        },
    }
}

fn host_terminal_command_builder(use_tmux_persistence: bool) -> CommandBuilder {
    if use_tmux_persistence {
        let mut cmd = CommandBuilder::new("tmux");
        host_terminal_apply_tmux_common_args(&mut cmd);
        cmd.args(["new-session", "-A", "-s", HOST_TERMINAL_SESSION_NAME]);
        host_terminal_apply_env(&mut cmd);
        if let Some(working_dir) = host_terminal_working_dir() {
            cmd.cwd(working_dir);
        }
        return cmd;
    }

    if cfg!(windows) {
        let comspec = std::env::var("COMSPEC")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "cmd.exe".to_string());
        let mut cmd = CommandBuilder::new(comspec);
        host_terminal_apply_env(&mut cmd);
        if let Some(working_dir) = host_terminal_working_dir() {
            cmd.cwd(working_dir);
        }
        return cmd;
    }

    let shell = std::env::var("SHELL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string());
    let mut cmd = CommandBuilder::new(shell);
    host_terminal_apply_env(&mut cmd);
    cmd.arg("-l");
    if let Some(working_dir) = host_terminal_working_dir() {
        cmd.cwd(working_dir);
    }
    cmd
}

fn spawn_host_terminal_runtime(
    cols: u16,
    rows: u16,
    use_tmux_persistence: bool,
    tmux_window_target: Option<&str>,
) -> TerminalResult<HostTerminalPtyRuntime> {
    if use_tmux_persistence {
        host_terminal_ensure_tmux_session()?;
        if let Some(target) = tmux_window_target {
            host_terminal_tmux_select_window(target)?;
        }
        host_terminal_tmux_reset_window_size(tmux_window_target);
    }
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: rows.max(1),
            cols: cols.max(2),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|err| format!("failed to allocate host PTY: {err}"))?;

    let portable_pty::PtyPair { master, slave } = pair;
    let cmd = host_terminal_command_builder(use_tmux_persistence);
    let child = slave
        .spawn_command(cmd)
        .map_err(|err| format!("failed to spawn host shell: {err}"))?;
    drop(slave);

    if use_tmux_persistence {
        host_terminal_apply_tmux_profile();
    }

    let writer = master
        .take_writer()
        .map_err(|err| format!("failed to open host terminal writer: {err}"))?;
    let reader = master
        .try_clone_reader()
        .map_err(|err| format!("failed to open host terminal reader: {err}"))?;
    let output_rx = spawn_host_terminal_reader(reader)?;

    Ok(HostTerminalPtyRuntime {
        master,
        writer,
        child,
        output_rx,
    })
}

fn spawn_host_terminal_reader(
    mut reader: Box<dyn Read + Send>,
) -> TerminalResult<tokio::sync::mpsc::UnboundedReceiver<HostTerminalOutputEvent>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<HostTerminalOutputEvent>();
    std::thread::Builder::new()
        .name("moltis-host-terminal-reader".to_string())
        .spawn(move || {
            let mut buf = vec![0_u8; 16 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        let _ = tx.send(HostTerminalOutputEvent::Closed);
                        break;
                    },
                    Ok(n) => {
                        if tx
                            .send(HostTerminalOutputEvent::Output(buf[..n].to_vec()))
                            .is_err()
                        {
                            return;
                        }
                    },
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(err) => {
                        let _ = tx.send(HostTerminalOutputEvent::Error(format!(
                            "host terminal stream error: {err}"
                        )));
                        let _ = tx.send(HostTerminalOutputEvent::Closed);
                        break;
                    },
                }
            }
        })
        .map_err(|err| format!("failed to launch host terminal reader thread: {err}"))?;
    Ok(rx)
}

fn host_terminal_write_input(
    runtime: &mut HostTerminalPtyRuntime,
    input: &str,
) -> TerminalResult<()> {
    runtime
        .writer
        .write_all(input.as_bytes())
        .map_err(|err| format!("failed to write to host terminal: {err}"))?;
    runtime
        .writer
        .flush()
        .map_err(|err| format!("failed to flush host terminal input: {err}"))?;
    Ok(())
}

fn host_terminal_resize(
    runtime: &HostTerminalPtyRuntime,
    cols: u16,
    rows: u16,
) -> TerminalResult<()> {
    let next_rows = rows.max(1);
    let next_cols = cols.max(2);
    runtime
        .master
        .resize(PtySize {
            rows: next_rows,
            cols: next_cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|err| format!("failed to resize host terminal: {err}"))?;
    Ok(())
}

fn host_terminal_stop_runtime(runtime: &mut HostTerminalPtyRuntime) {
    let _ = runtime.child.kill();
}

fn detect_host_root_user_for_terminal() -> Option<bool> {
    if cfg!(windows) {
        return None;
    }

    if let Some(uid) = std::env::var("EUID")
        .ok()
        .or_else(|| std::env::var("UID").ok())
        .and_then(|value| value.trim().parse::<u32>().ok())
    {
        return Some(uid == 0);
    }

    if let Some(user) = std::env::var("USER")
        .ok()
        .or_else(|| std::env::var("LOGNAME").ok())
    {
        let trimmed = user.trim();
        if !trimmed.is_empty() {
            return Some(trimmed == "root");
        }
    }

    None
}

// ── Auth / origin helpers (needed by terminal WS handler) ────────────────────

/// Returns `true` when the request carries headers typically set by reverse proxies.
fn has_proxy_headers(headers: &axum::http::HeaderMap) -> bool {
    headers.contains_key("x-forwarded-for")
        || headers.contains_key("x-real-ip")
        || headers.contains_key("cf-connecting-ip")
        || headers.get("forwarded").is_some()
}

/// Returns `true` when `host` (without port) is a loopback name/address.
fn is_loopback_host(host: &str) -> bool {
    // Strip port (IPv6 bracket form, bare IPv6, or simple host:port).
    let name = if host.starts_with('[') {
        // [::1]:port or [::1]
        host.rsplit_once("]:")
            .map_or(host, |(addr, _)| addr)
            .trim_start_matches('[')
            .trim_end_matches(']')
    } else if host.matches(':').count() > 1 {
        // Bare IPv6 like ::1 (multiple colons, no brackets) — no port stripping.
        host
    } else {
        host.rsplit_once(':').map_or(host, |(addr, _)| addr)
    };
    matches!(name, "localhost" | "127.0.0.1" | "::1") || name.ends_with(".localhost")
}

/// Determine whether a connection is a **direct local** connection (no proxy
/// in between).  This is the per-request check used by the three-tier auth
/// model:
///
/// 1. Password set -> always require auth
/// 2. No password + local -> full access (dev convenience)
/// 3. No password + remote/proxied -> onboarding only
///
/// A connection is considered local when **all** of the following hold:
///
/// - `MOLTIS_BEHIND_PROXY` is **not** set (`behind_proxy == false`)
/// - No proxy headers are present (X-Forwarded-For, X-Real-IP, etc.)
/// - The `Host` header resolves to a loopback address (or is absent)
/// - The TCP source IP is loopback
fn is_local_connection(
    headers: &axum::http::HeaderMap,
    remote_addr: SocketAddr,
    behind_proxy: bool,
) -> bool {
    // Hard override: env var says we're behind a proxy.
    if behind_proxy {
        return false;
    }

    // Proxy headers present -> proxied traffic.
    if has_proxy_headers(headers) {
        return false;
    }

    // Host header points to a non-loopback name -> likely proxied.
    if let Some(host) = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        && !is_loopback_host(host)
    {
        return false;
    }

    // TCP source must be loopback.
    remote_addr.ip().is_loopback()
}

async fn websocket_header_authenticated(
    headers: &axum::http::HeaderMap,
    credential_store: Option<&Arc<moltis_gateway::auth::CredentialStore>>,
    is_local: bool,
) -> bool {
    let Some(store) = credential_store else {
        return false;
    };

    matches!(
        moltis_gateway::auth_middleware::check_auth(store, headers, is_local).await,
        moltis_gateway::auth_middleware::AuthResult::Allowed(_)
    )
}

/// Check whether a WebSocket `Origin` header matches the request `Host`.
///
/// Extracts the host portion of the origin URL and compares it to the Host
/// header.  Accepts `localhost`, `127.0.0.1`, and `[::1]` interchangeably
/// so that `http://localhost:8080` matches a Host of `127.0.0.1:8080`.
fn is_same_origin(origin: &str, host: &str) -> bool {
    // Origin is a full URL (e.g. "https://localhost:8080"), Host is just
    // "host:port" or "host".
    let origin_host = origin
        .split("://")
        .nth(1)
        .unwrap_or(origin)
        .split('/')
        .next()
        .unwrap_or("");

    fn strip_port(h: &str) -> &str {
        if h.starts_with('[') {
            // IPv6: [::1]:port
            h.rsplit_once("]:")
                .map_or(h, |(addr, _)| addr)
                .trim_start_matches('[')
                .trim_end_matches(']')
        } else {
            h.rsplit_once(':').map_or(h, |(addr, _)| addr)
        }
    }
    fn get_port(h: &str) -> Option<&str> {
        if h.starts_with('[') {
            h.rsplit_once("]:").map(|(_, p)| p)
        } else {
            h.rsplit_once(':').map(|(_, p)| p)
        }
    }

    let origin_port = get_port(origin_host);
    let host_port = get_port(host);

    let oh = strip_port(origin_host);
    let hh = strip_port(host);

    // Normalise loopback variants so 127.0.0.1 == localhost == ::1.
    // Subdomains of .localhost (e.g. moltis.localhost) are also loopback per RFC 6761.
    let is_loopback =
        |h: &str| matches!(h, "localhost" | "127.0.0.1" | "::1") || h.ends_with(".localhost");

    (oh == hh || (is_loopback(oh) && is_loopback(hh))) && origin_port == host_port
}

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

pub async fn api_terminal_windows_handler() -> impl IntoResponse {
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
    Json(payload): Json<HostTerminalCreateWindowRequest>,
) -> impl IntoResponse {
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

// ── WebSocket helpers ────────────────────────────────────────────────────────

async fn terminal_ws_send_json(
    ws_tx: &mut futures::stream::SplitSink<WebSocket, Message>,
    payload: serde_json::Value,
) -> bool {
    match serde_json::to_string(&payload) {
        Ok(text) => ws_tx.send(Message::Text(text.into())).await.is_ok(),
        Err(err) => {
            warn!(error = %err, "failed to serialize terminal ws payload");
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

async fn handle_terminal_ws_connection(
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
