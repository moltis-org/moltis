use std::process::Command;

use {portable_pty::CommandBuilder, tracing::debug};

use super::types::{
    HOST_TERMINAL_SESSION_NAME, HOST_TERMINAL_TMUX_CONFIG_PATH, HOST_TERMINAL_TMUX_SOCKET_NAME,
    HostTerminalWindowInfo, TerminalResult, host_terminal_working_dir,
};

// ── tmux availability and install hints ──────────────────────────────────────

pub(crate) fn host_terminal_tmux_available() -> bool {
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

pub(crate) fn host_terminal_tmux_install_hint() -> Option<String> {
    tmux_install_command_for_host_os().map(str::to_string)
}

// ── tmux command builders ────────────────────────────────────────────────────

pub(crate) fn host_terminal_apply_env(cmd: &mut CommandBuilder) {
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("TMUX", "");
}

pub(crate) fn host_terminal_apply_tmux_common_args(cmd: &mut CommandBuilder) {
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

// ── tmux profile ─────────────────────────────────────────────────────────────

pub(crate) fn host_terminal_apply_tmux_profile() {
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

// ── Window name/target helpers ───────────────────────────────────────────────

pub(crate) fn host_terminal_normalize_window_name(name: &str) -> TerminalResult<String> {
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

pub(crate) fn host_terminal_resolve_window_target(
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

pub(crate) fn host_terminal_default_window_target(
    windows: &[HostTerminalWindowInfo],
) -> Option<String> {
    windows
        .iter()
        .find(|window| window.active)
        .or_else(|| windows.first())
        .map(|window| window.id.clone())
}

// ── Session management ───────────────────────────────────────────────────────

pub(crate) fn host_terminal_ensure_tmux_session() -> TerminalResult<()> {
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

// ── Window parsing ───────────────────────────────────────────────────────────

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

pub(crate) fn host_terminal_tmux_list_windows() -> TerminalResult<Vec<HostTerminalWindowInfo>> {
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

// ── Window creation / selection / resize ─────────────────────────────────────

pub(crate) fn host_terminal_tmux_create_window(name: Option<&str>) -> TerminalResult<String> {
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

pub(crate) fn host_terminal_tmux_select_window(window_target: &str) -> TerminalResult<()> {
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

pub(crate) fn host_terminal_tmux_reset_window_size(window_target: Option<&str>) {
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
