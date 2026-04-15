use std::{io::Write, path::PathBuf};

// ── Constants ────────────────────────────────────────────────────────────────

pub(crate) const HOST_TERMINAL_SESSION_NAME: &str = "moltis-host-terminal";
pub(crate) const HOST_TERMINAL_TMUX_SOCKET_NAME: &str = "moltis-host-terminal";
pub(crate) const HOST_TERMINAL_TMUX_CONFIG_PATH: &str = "/dev/null";
pub(crate) const HOST_TERMINAL_MAX_INPUT_BYTES: usize = 8 * 1024;
pub(crate) const HOST_TERMINAL_DEFAULT_COLS: u16 = 220;
pub(crate) const HOST_TERMINAL_DEFAULT_ROWS: u16 = 56;
pub(crate) const TERMINAL_SESSION_INIT_FAILED: &str = "TERMINAL_SESSION_INIT_FAILED";
pub(crate) const TERMINAL_WINDOWS_LIST_FAILED: &str = "TERMINAL_WINDOWS_LIST_FAILED";
pub(crate) const TERMINAL_TMUX_UNAVAILABLE: &str = "TERMINAL_TMUX_UNAVAILABLE";
pub(crate) const TERMINAL_WINDOW_NAME_INVALID: &str = "TERMINAL_WINDOW_NAME_INVALID";
pub(crate) const TERMINAL_WINDOW_CREATE_FAILED: &str = "TERMINAL_WINDOW_CREATE_FAILED";
pub(crate) const TERMINAL_DISABLED: &str = "TERMINAL_DISABLED";

// ── Data structures ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct HostTerminalWsQuery {
    pub(crate) window: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct HostTerminalWindowInfo {
    pub(crate) id: String,
    pub(crate) index: u32,
    pub(crate) name: String,
    pub(crate) active: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct HostTerminalCreateWindowRequest {
    #[serde(default)]
    pub(crate) name: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum HostTerminalWsClientMessage {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
    SwitchWindow { window: String },
    Control { action: HostTerminalWsControlAction },
    Ping,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HostTerminalWsControlAction {
    Restart,
    CtrlC,
    Clear,
}

pub(crate) enum HostTerminalOutputEvent {
    Output(Vec<u8>),
    Error(String),
    Closed,
}

pub(crate) struct HostTerminalPtyRuntime {
    pub(crate) master: Box<dyn portable_pty::MasterPty + Send>,
    pub(crate) writer: Box<dyn Write + Send>,
    pub(crate) child: Box<dyn portable_pty::Child + Send + Sync>,
    pub(crate) output_rx: tokio::sync::mpsc::UnboundedReceiver<HostTerminalOutputEvent>,
}

// ── Error type ───────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct TerminalError {
    pub(crate) message: String,
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

pub(crate) type TerminalResult<T> = Result<T, TerminalError>;

// ── Shared helpers ───────────────────────────────────────────────────────────

pub(crate) fn terminal_error(code: &str, error: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "code": code,
        "error": error.into(),
    })
}

pub(crate) fn host_terminal_working_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
}

pub(crate) fn host_terminal_user_name() -> String {
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

pub(crate) fn detect_host_root_user_for_terminal() -> Option<bool> {
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
