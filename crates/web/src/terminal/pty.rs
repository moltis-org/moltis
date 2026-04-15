use std::io::{Read, Write};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use super::types::{
    HOST_TERMINAL_SESSION_NAME, HostTerminalOutputEvent, HostTerminalPtyRuntime, TerminalResult,
    host_terminal_working_dir,
};

use super::tmux::{
    host_terminal_apply_env, host_terminal_apply_tmux_common_args,
    host_terminal_apply_tmux_profile, host_terminal_ensure_tmux_session,
    host_terminal_tmux_reset_window_size, host_terminal_tmux_select_window,
};

// ── Command builder ──────────────────────────────────────────────────────────

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

// ── PTY runtime spawn / control ──────────────────────────────────────────────

pub(crate) fn spawn_host_terminal_runtime(
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

pub(crate) fn host_terminal_write_input(
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

pub(crate) fn host_terminal_resize(
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

pub(crate) fn host_terminal_stop_runtime(runtime: &mut HostTerminalPtyRuntime) {
    let _ = runtime.child.kill();
}
