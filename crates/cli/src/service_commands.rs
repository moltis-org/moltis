//! `moltis service` subcommands — install/manage the gateway as an OS service.
//!
//! - **macOS**: launchd user agent (`~/Library/LaunchAgents/org.moltis.plist`)
//! - **Linux**: systemd user unit (`~/.config/systemd/user/moltis.service`)
//! - **Linux without systemd**: portable user supervisor (`~/.moltis/moltis-service-supervisor.sh`)

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use {anyhow::Result, clap::Subcommand};

/// `moltis service` subcommands.
#[derive(Subcommand)]
pub enum ServiceAction {
    /// Install moltis as an OS service (launchd on macOS, systemd on Linux).
    Install {
        /// Address to bind to (passed as --bind).
        #[arg(long)]
        bind: Option<String>,
        /// Port to listen on (passed as --port).
        #[arg(long)]
        port: Option<u16>,
        /// Log level for the service.
        #[arg(long, default_value = "info")]
        log_level: String,
    },

    /// Uninstall the moltis service.
    Uninstall,

    /// Show the current status of the moltis service.
    Status,

    /// Stop the moltis service.
    Stop,

    /// Restart the moltis service.
    Restart,

    /// Print the path to the service log file.
    Logs,
}

pub fn handle_service(action: ServiceAction) -> Result<()> {
    match action {
        ServiceAction::Install {
            bind,
            port,
            log_level,
        } => {
            let data_dir = moltis_config::data_dir();
            let log_path = data_dir.join("moltis.log");
            let moltis_bin = resolve_binary()?;

            let opts = GatewayServiceOpts {
                bind,
                port,
                log_level,
            };

            if cfg!(target_os = "macos") {
                install_launchd(&moltis_bin, &opts, &log_path)?;
            } else if cfg!(target_os = "linux") {
                if systemd_user_available() {
                    install_systemd(&moltis_bin, &opts, &log_path)?;
                } else {
                    install_process_service(&moltis_bin, &opts, &log_path)?;
                    println!("Using portable supervisor because systemd --user is unavailable.");
                    println!(
                        "Auto-start after container reboot requires your devbox startup configuration."
                    );
                }
            } else {
                anyhow::bail!("service install not supported on {}", std::env::consts::OS);
            }

            println!("Moltis service installed and started.");
            println!("Logs: {}", log_path.display());
            Ok(())
        },

        ServiceAction::Uninstall => {
            if cfg!(target_os = "macos") {
                uninstall_launchd()?;
            } else if cfg!(target_os = "linux") {
                uninstall_linux_service()?;
            } else {
                anyhow::bail!(
                    "service uninstall not supported on {}",
                    std::env::consts::OS
                );
            }
            println!("Moltis service uninstalled.");
            Ok(())
        },

        ServiceAction::Status => {
            let status = if cfg!(target_os = "macos") {
                status_launchd()?
            } else if cfg!(target_os = "linux") {
                status_linux_service()?
            } else {
                anyhow::bail!("service status not supported on {}", std::env::consts::OS);
            };
            println!("Moltis service: {status}");
            Ok(())
        },

        ServiceAction::Stop => {
            if cfg!(target_os = "macos") {
                stop_launchd()?;
            } else if cfg!(target_os = "linux") {
                stop_linux_service()?;
            } else {
                anyhow::bail!("service stop not supported on {}", std::env::consts::OS);
            }
            println!("Moltis service stopped.");
            Ok(())
        },

        ServiceAction::Restart => {
            if cfg!(target_os = "macos") {
                restart_launchd()?;
            } else if cfg!(target_os = "linux") {
                restart_linux_service()?;
            } else {
                anyhow::bail!("service restart not supported on {}", std::env::consts::OS);
            }
            println!("Moltis service restarted.");
            Ok(())
        },

        ServiceAction::Logs => {
            let data_dir = moltis_config::data_dir();
            println!("{}", data_dir.join("moltis.log").display());
            Ok(())
        },
    }
}

// ── Types ──────────────────────────────────────────────────────────────────

struct GatewayServiceOpts {
    bind: Option<String>,
    port: Option<u16>,
    log_level: String,
}

// ── Status ─────────────────────────────────────────────────────────────────

enum ServiceStatus {
    Running { pid: Option<u32> },
    Stopped,
    NotInstalled,
    Unknown(String),
}

impl std::fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running { pid: Some(p) } => write!(f, "running (pid {p})"),
            Self::Running { pid: None } => write!(f, "running"),
            Self::Stopped => write!(f, "stopped"),
            Self::NotInstalled => write!(f, "not installed"),
            Self::Unknown(message) => write!(f, "unknown: {message}"),
        }
    }
}

// ── Binary resolution ──────────────────────────────────────────────────────

fn resolve_binary() -> Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        let name = exe.file_name().unwrap_or_default().to_string_lossy();
        if name == "moltis" || name.starts_with("moltis-") {
            return Ok(exe);
        }
    }

    which::which("moltis").map_err(|_| {
        anyhow::anyhow!("cannot find 'moltis' binary; ensure it is installed and in PATH")
    })
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory (HOME not set)"))
}

fn uid() -> u32 {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(501)
}

// ── macOS launchd ──────────────────────────────────────────────────────────

const LAUNCHD_LABEL: &str = "org.moltis.gateway";
const SYSTEMD_UNIT: &str = "moltis.service";

fn launchd_plist_path() -> Result<PathBuf> {
    let home = home_dir()?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LAUNCHD_LABEL}.plist")))
}

fn generate_launchd_plist(moltis_bin: &Path, opts: &GatewayServiceOpts, log_path: &Path) -> String {
    let bin = moltis_bin.display();
    let log = log_path.display();

    let mut args = vec![
        format!("    <string>{bin}</string>"),
        "    <string>--log-level</string>".to_string(),
        format!("    <string>{}</string>", opts.log_level),
    ];

    if let Some(ref bind) = opts.bind {
        args.push("    <string>--bind</string>".to_string());
        args.push(format!("    <string>{bind}</string>"));
    }
    if let Some(port) = opts.port {
        args.push("    <string>--port</string>".to_string());
        args.push(format!("    <string>{port}</string>"));
    }

    let args_str = args.join("\n");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{args_str}
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>ThrottleInterval</key>
  <integer>10</integer>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
  <key>ProcessType</key>
  <string>Background</string>
</dict>
</plist>
"#
    )
}

fn install_launchd(moltis_bin: &Path, opts: &GatewayServiceOpts, log_path: &Path) -> Result<()> {
    let plist_path = launchd_plist_path()?;

    // Unload first if already loaded (ignore errors).
    let _ = Command::new("launchctl")
        .args([
            "bootout",
            &format!("gui/{}", uid()),
            plist_path.to_str().unwrap_or_default(),
        ])
        .output();

    let plist = generate_launchd_plist(moltis_bin, opts, log_path);

    if let Some(parent) = plist_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&plist_path, &plist)?;

    let output = Command::new("launchctl")
        .args([
            "bootstrap",
            &format!("gui/{}", uid()),
            plist_path.to_str().unwrap_or_default(),
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("launchctl bootstrap failed: {stderr}");
    }

    Ok(())
}

fn uninstall_launchd() -> Result<()> {
    let plist_path = launchd_plist_path()?;
    if !plist_path.exists() {
        anyhow::bail!("service not installed (plist not found)");
    }

    let _ = Command::new("launchctl")
        .args([
            "bootout",
            &format!("gui/{}", uid()),
            plist_path.to_str().unwrap_or_default(),
        ])
        .output();

    fs::remove_file(&plist_path)?;
    Ok(())
}

fn status_launchd() -> Result<ServiceStatus> {
    let plist_path = launchd_plist_path()?;
    if !plist_path.exists() {
        return Ok(ServiceStatus::NotInstalled);
    }

    let output = Command::new("launchctl")
        .args(["print", &format!("gui/{}/{LAUNCHD_LABEL}", uid())])
        .output()?;

    if !output.status.success() {
        return Ok(ServiceStatus::Stopped);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let pid = stdout.lines().find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .starts_with("pid = ")
            .then(|| trimmed.strip_prefix("pid = ")?.parse::<u32>().ok())
            .flatten()
    });

    Ok(ServiceStatus::Running { pid })
}

fn stop_launchd() -> Result<()> {
    let plist_path = launchd_plist_path()?;
    if !plist_path.exists() {
        anyhow::bail!("service not installed");
    }

    let output = Command::new("launchctl")
        .args(["kill", "SIGTERM", &format!("gui/{}/{LAUNCHD_LABEL}", uid())])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("No such process") && !stderr.contains("3: No such process") {
            anyhow::bail!("launchctl kill failed: {stderr}");
        }
    }

    Ok(())
}

fn restart_launchd() -> Result<()> {
    let plist_path = launchd_plist_path()?;
    if !plist_path.exists() {
        anyhow::bail!("service not installed");
    }

    let output = Command::new("launchctl")
        .args(["kickstart", "-k", &format!("gui/{}/{LAUNCHD_LABEL}", uid())])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("launchctl kickstart failed: {stderr}");
    }

    Ok(())
}

// ── Linux systemd ──────────────────────────────────────────────────────────

fn systemd_unit_path() -> Result<PathBuf> {
    let home = home_dir()?;
    Ok(home
        .join(".config")
        .join("systemd")
        .join("user")
        .join(SYSTEMD_UNIT))
}

fn generate_systemd_unit(moltis_bin: &Path, opts: &GatewayServiceOpts, log_path: &Path) -> String {
    let bin = moltis_bin.display();
    let log = log_path.display();

    let mut exec_args = format!("{bin} --log-level {}", opts.log_level);

    if let Some(ref bind) = opts.bind {
        exec_args.push_str(&format!(" --bind {bind}"));
    }
    if let Some(port) = opts.port {
        exec_args.push_str(&format!(" --port {port}"));
    }

    format!(
        r#"[Unit]
Description=Moltis Gateway
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={exec_args}
Restart=on-failure
RestartSec=10
StandardOutput=append:{log}
StandardError=append:{log}
Environment=RUST_LOG=info

[Install]
WantedBy=default.target
"#
    )
}

fn uninstall_linux_service() -> Result<()> {
    let mut removed = false;
    let supervisor_path = process_service_supervisor_path()?;

    if supervisor_path.exists() {
        stop_process_service_if_installed()?;
        fs::remove_file(supervisor_path)?;
        remove_file_if_exists(&process_service_pid_path()?)?;
        remove_file_if_exists(&process_service_child_pid_path()?)?;
        removed = true;
    }

    if systemd_unit_path()?.exists() {
        uninstall_systemd()?;
        removed = true;
    }

    if !removed {
        anyhow::bail!("service not installed");
    }

    Ok(())
}

fn status_linux_service() -> Result<ServiceStatus> {
    if process_service_supervisor_path()?.exists() {
        return status_process_service();
    }

    status_systemd()
}

fn stop_linux_service() -> Result<()> {
    if process_service_supervisor_path()?.exists() {
        return stop_process_service();
    }

    if !systemd_user_available() {
        anyhow::bail!(portable_service_not_installed_message());
    }

    stop_systemd()
}

fn restart_linux_service() -> Result<()> {
    if process_service_supervisor_path()?.exists() {
        return restart_process_service();
    }

    if !systemd_user_available() {
        anyhow::bail!(portable_service_not_installed_message());
    }

    restart_systemd()
}

fn portable_service_not_installed_message() -> &'static str {
    "service not installed; systemd --user is unavailable, so run `moltis service install` to use the portable supervisor"
}

fn install_systemd(moltis_bin: &Path, opts: &GatewayServiceOpts, log_path: &Path) -> Result<()> {
    let unit_path = systemd_unit_path()?;

    let _ = Command::new("systemctl")
        .args(["--user", "stop", SYSTEMD_UNIT])
        .output();

    let unit = generate_systemd_unit(moltis_bin, opts, log_path);

    if let Some(parent) = unit_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&unit_path, &unit)?;

    run_systemctl(&["daemon-reload"])?;
    run_systemctl(&["enable", SYSTEMD_UNIT])?;
    run_systemctl(&["start", SYSTEMD_UNIT])?;

    Ok(())
}

fn uninstall_systemd() -> Result<()> {
    let unit_path = systemd_unit_path()?;
    if !unit_path.exists() {
        anyhow::bail!("service not installed (unit file not found)");
    }

    let _ = run_systemctl(&["stop", SYSTEMD_UNIT]);
    let _ = run_systemctl(&["disable", SYSTEMD_UNIT]);
    fs::remove_file(&unit_path)?;
    let _ = run_systemctl(&["daemon-reload"]);

    Ok(())
}

fn status_systemd() -> Result<ServiceStatus> {
    let unit_path = systemd_unit_path()?;
    if !unit_path.exists() {
        return Ok(ServiceStatus::NotInstalled);
    }

    let output = Command::new("systemctl")
        .args(["--user", "is-active", SYSTEMD_UNIT])
        .output()?;

    let output_text = command_output_text(&output);
    if systemd_unavailable_output(&output_text) {
        return Ok(ServiceStatus::Unknown(
            "systemd --user is unavailable; run `moltis service install` to use the portable supervisor".into(),
        ));
    }

    let state = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() && state.is_empty() {
        return Ok(ServiceStatus::Unknown(format!(
            "systemctl is-active failed: {output_text}"
        )));
    }

    match state.as_str() {
        "active" => {
            let pid_output = Command::new("systemctl")
                .args([
                    "--user",
                    "show",
                    SYSTEMD_UNIT,
                    "--property=MainPID",
                    "--value",
                ])
                .output()?;
            let pid = String::from_utf8_lossy(&pid_output.stdout)
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|p| *p > 0);
            Ok(ServiceStatus::Running { pid })
        },
        "inactive" | "deactivating" => Ok(ServiceStatus::Stopped),
        "failed" => Ok(ServiceStatus::Unknown("failed".into())),
        other => Ok(ServiceStatus::Unknown(other.into())),
    }
}

fn stop_systemd() -> Result<()> {
    let unit_path = systemd_unit_path()?;
    if !unit_path.exists() {
        anyhow::bail!("service not installed");
    }
    run_systemctl(&["stop", SYSTEMD_UNIT])
}

fn restart_systemd() -> Result<()> {
    let unit_path = systemd_unit_path()?;
    if !unit_path.exists() {
        anyhow::bail!("service not installed");
    }
    run_systemctl(&["restart", SYSTEMD_UNIT])
}

fn run_systemctl(args: &[&str]) -> Result<()> {
    let mut full_args = vec!["--user"];
    full_args.extend_from_slice(args);

    let output = Command::new("systemctl").args(&full_args).output()?;
    let output_text = command_output_text(&output);
    if systemd_unavailable_output(&output_text) {
        anyhow::bail!(
            "systemctl {} failed because systemd --user is unavailable in this environment: {output_text}",
            args.join(" ")
        );
    }

    if !output.status.success() {
        anyhow::bail!("systemctl {} failed: {output_text}", args.join(" "));
    }
    Ok(())
}

fn systemd_user_available() -> bool {
    Command::new("systemctl")
        .args([
            "--user",
            "list-units",
            "--type=service",
            "--no-pager",
            "--no-legend",
        ])
        .output()
        .map(|output| {
            output.status.success() && !systemd_unavailable_output(&command_output_text(&output))
        })
        .unwrap_or(false)
}

fn command_output_text(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => format!("exit status {}", output.status),
        (false, true) => stdout,
        (true, false) => stderr,
        (false, false) => format!("{stdout}\n{stderr}"),
    }
}

fn systemd_unavailable_output(output: &str) -> bool {
    let output = output.to_ascii_lowercase();
    (output.contains("systemd")
        && (output.contains("not running") || output.contains("not been booted")))
        || output.contains("failed to connect to bus")
        || output.contains("no medium found")
}

// ── Linux portable process supervisor ──────────────────────────────────────

fn process_service_supervisor_path() -> Result<PathBuf> {
    Ok(moltis_config::data_dir().join("moltis-service-supervisor.sh"))
}

fn process_service_pid_path() -> Result<PathBuf> {
    Ok(moltis_config::data_dir().join("moltis-service.pid"))
}

fn process_service_child_pid_path() -> Result<PathBuf> {
    Ok(moltis_config::data_dir().join("moltis.pid"))
}

fn process_service_stop_path() -> Result<PathBuf> {
    Ok(moltis_config::data_dir().join("moltis-service.stop"))
}

fn install_process_service(
    moltis_bin: &Path,
    opts: &GatewayServiceOpts,
    log_path: &Path,
) -> Result<()> {
    stop_process_service_if_installed()?;

    let supervisor_path = process_service_supervisor_path()?;
    if let Some(parent) = supervisor_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let script = generate_process_supervisor_script(moltis_bin, opts, log_path)?;
    fs::write(&supervisor_path, script)?;

    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(&supervisor_path)?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&supervisor_path, permissions)?;
    }

    start_process_service()
}

fn stop_process_service_if_installed() -> Result<()> {
    if process_service_supervisor_path()?.exists() {
        stop_process_service()?;
    }
    Ok(())
}

fn status_process_service() -> Result<ServiceStatus> {
    let supervisor_path = process_service_supervisor_path()?;
    if !supervisor_path.exists() {
        return Ok(ServiceStatus::NotInstalled);
    }

    let child_pid = read_pid(&process_service_child_pid_path()?)?;
    if child_pid.is_some_and(pid_is_alive) {
        return Ok(ServiceStatus::Running { pid: child_pid });
    }

    let supervisor_pid = read_pid(&process_service_pid_path()?)?;
    if supervisor_pid.is_some_and(pid_is_alive) {
        return Ok(ServiceStatus::Running {
            pid: supervisor_pid,
        });
    }

    Ok(ServiceStatus::Stopped)
}

fn stop_process_service() -> Result<()> {
    if !process_service_supervisor_path()?.exists() {
        anyhow::bail!("service not installed");
    }

    fs::write(process_service_stop_path()?, b"stop\n")?;

    let child_pid = read_pid(&process_service_child_pid_path()?)?;
    let supervisor_pid = read_pid(&process_service_pid_path()?)?;

    if let Some(pid) = child_pid.filter(|pid| pid_is_alive(*pid)) {
        let _ = signal_process(pid, "TERM");
    }
    if let Some(pid) = supervisor_pid.filter(|pid| pid_is_alive(*pid)) {
        let _ = signal_process(pid, "TERM");
        wait_for_process_exit(pid);
        if pid_is_alive(pid) {
            let _ = signal_process(pid, "KILL");
            wait_for_process_exit(pid);
        }
    }
    if let Some(pid) = child_pid.filter(|pid| pid_is_alive(*pid)) {
        wait_for_process_exit(pid);
        if pid_is_alive(pid) {
            let _ = signal_process(pid, "KILL");
            wait_for_process_exit(pid);
        }
    }

    remove_file_if_exists(&process_service_pid_path()?)?;
    remove_file_if_exists(&process_service_child_pid_path()?)?;
    remove_file_if_exists(&process_service_stop_path()?)?;

    Ok(())
}

fn restart_process_service() -> Result<()> {
    if !process_service_supervisor_path()?.exists() {
        anyhow::bail!("service not installed");
    }

    stop_process_service()?;
    start_process_service()
}

fn start_process_service() -> Result<()> {
    let supervisor_path = process_service_supervisor_path()?;
    if !supervisor_path.exists() {
        anyhow::bail!("service not installed");
    }

    remove_file_if_exists(&process_service_stop_path()?)?;
    remove_file_if_exists(&process_service_pid_path()?)?;
    remove_file_if_exists(&process_service_child_pid_path()?)?;

    let mut command = if which::which("setsid").is_ok() {
        let mut command = Command::new("setsid");
        command.arg(&supervisor_path);
        command
    } else {
        Command::new(&supervisor_path)
    };

    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    wait_for_process_service_start()
}

fn wait_for_process_service_start() -> Result<()> {
    for _ in 0..50 {
        match status_process_service()? {
            ServiceStatus::Running { .. } => return Ok(()),
            ServiceStatus::Stopped | ServiceStatus::NotInstalled | ServiceStatus::Unknown(_) => {},
        }
        thread::sleep(Duration::from_millis(100));
    }

    anyhow::bail!(
        "portable supervisor did not start; check logs with `tail -f {}`",
        moltis_config::data_dir().join("moltis.log").display()
    )
}

fn generate_process_supervisor_script(
    moltis_bin: &Path,
    opts: &GatewayServiceOpts,
    log_path: &Path,
) -> Result<String> {
    let data_dir = moltis_config::data_dir();
    let supervisor_pid_path = process_service_pid_path()?;
    let child_pid_path = process_service_child_pid_path()?;
    let stop_path = process_service_stop_path()?;

    let mut args = vec!["--log-level".to_string(), opts.log_level.clone()];
    if let Some(bind) = &opts.bind {
        args.push("--bind".to_string());
        args.push(bind.clone());
    }
    if let Some(port) = opts.port {
        args.push("--port".to_string());
        args.push(port.to_string());
    }

    let command_args = args
        .iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ");

    Ok(format!(
        r#"#!/bin/sh
set -u

MOLTIS_BIN={bin}
DATA_DIR={data_dir}
LOG_FILE={log}
SUPERVISOR_PID_FILE={supervisor_pid}
CHILD_PID_FILE={child_pid}
STOP_FILE={stop_file}
RESTART_DELAY=10

mkdir -p "$DATA_DIR"
rm -f "$STOP_FILE"
printf '%s\n' "$$" > "$SUPERVISOR_PID_FILE"

cleanup() {{
  touch "$STOP_FILE"
  if [ -s "$CHILD_PID_FILE" ]; then
    kill "$(cat "$CHILD_PID_FILE")" 2>/dev/null || true
  fi
  rm -f "$SUPERVISOR_PID_FILE" "$CHILD_PID_FILE"
  exit 0
}}

trap cleanup INT TERM

while [ ! -e "$STOP_FILE" ]; do
  RUST_LOG=info "$MOLTIS_BIN" {args} >> "$LOG_FILE" 2>&1 &
  child="$!"
  printf '%s\n' "$child" > "$CHILD_PID_FILE"
  wait "$child"
  status="$?"
  rm -f "$CHILD_PID_FILE"

  if [ -e "$STOP_FILE" ]; then
    break
  fi

  printf '%s\n' "moltis exited with status $status; restarting in $RESTART_DELAY seconds" >> "$LOG_FILE"
  sleep "$RESTART_DELAY" &
  wait "$!"
done

rm -f "$SUPERVISOR_PID_FILE" "$CHILD_PID_FILE" "$STOP_FILE"
"#,
        bin = shell_quote(&moltis_bin.display().to_string()),
        args = command_args,
        data_dir = shell_quote(&data_dir.display().to_string()),
        log = shell_quote(&log_path.display().to_string()),
        supervisor_pid = shell_quote(&supervisor_pid_path.display().to_string()),
        child_pid = shell_quote(&child_pid_path.display().to_string()),
        stop_file = shell_quote(&stop_path.display().to_string()),
    ))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn read_pid(path: &Path) -> Result<Option<u32>> {
    if !path.exists() {
        return Ok(None);
    }

    let contents = fs::read_to_string(path)?;
    Ok(contents.trim().parse::<u32>().ok())
}

fn pid_is_alive(pid: u32) -> bool {
    if is_linux_zombie(pid) {
        return false;
    }

    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .is_ok_and(|status| status.success())
}

fn is_linux_zombie(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| linux_proc_stat_state(&stat))
        .is_some_and(|state| state == 'Z')
}

fn linux_proc_stat_state(stat: &str) -> Option<char> {
    stat.rsplit_once(") ")?
        .1
        .split_whitespace()
        .next()?
        .chars()
        .next()
}

fn signal_process(pid: u32, signal: &str) -> bool {
    Command::new("kill")
        .args([format!("-{signal}"), pid.to_string()])
        .status()
        .is_ok_and(|status| status.success())
}

fn wait_for_process_exit(pid: u32) {
    for _ in 0..30 {
        if !pid_is_alive(pid) {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn launchd_plist_basic() {
        let bin = PathBuf::from("/opt/homebrew/bin/moltis");
        let opts = GatewayServiceOpts {
            bind: None,
            port: None,
            log_level: "info".into(),
        };
        let log = PathBuf::from("/tmp/moltis.log");

        let plist = generate_launchd_plist(&bin, &opts, &log);

        assert!(plist.starts_with("<?xml"));
        assert!(plist.contains("org.moltis.gateway"));
        assert!(plist.contains("/opt/homebrew/bin/moltis"));
        assert!(plist.contains("--log-level"));
        assert!(plist.contains("info"));
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
        assert!(plist.contains("/tmp/moltis.log"));
        assert!(plist.contains("</plist>"));
        // Args should NOT include a "gateway" subcommand — just `moltis` directly.
        assert!(!plist.contains("<string>gateway</string>"));
    }

    #[test]
    fn launchd_plist_with_bind_and_port() {
        let bin = PathBuf::from("/usr/local/bin/moltis");
        let opts = GatewayServiceOpts {
            bind: Some("0.0.0.0".into()),
            port: Some(8080),
            log_level: "debug".into(),
        };
        let log = PathBuf::from("/tmp/moltis.log");

        let plist = generate_launchd_plist(&bin, &opts, &log);

        assert!(plist.contains("--bind"));
        assert!(plist.contains("0.0.0.0"));
        assert!(plist.contains("--port"));
        assert!(plist.contains("8080"));
        assert!(plist.contains("--log-level"));
        assert!(plist.contains("debug"));
    }

    #[test]
    fn launchd_plist_omits_optional_flags() {
        let bin = PathBuf::from("/usr/local/bin/moltis");
        let opts = GatewayServiceOpts {
            bind: None,
            port: None,
            log_level: "info".into(),
        };
        let log = PathBuf::from("/tmp/moltis.log");

        let plist = generate_launchd_plist(&bin, &opts, &log);

        assert!(!plist.contains("--bind"));
        assert!(!plist.contains("--port"));
    }

    #[test]
    fn systemd_unit_basic() {
        let bin = PathBuf::from("/usr/bin/moltis");
        let opts = GatewayServiceOpts {
            bind: None,
            port: None,
            log_level: "info".into(),
        };
        let log = PathBuf::from("/var/log/moltis.log");

        let unit = generate_systemd_unit(&bin, &opts, &log);

        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Service]"));
        assert!(unit.contains("[Install]"));
        assert!(unit.contains("Moltis Gateway"));
        assert!(unit.contains("/usr/bin/moltis --log-level info"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("RestartSec=10"));
        assert!(unit.contains("/var/log/moltis.log"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn systemd_unit_with_bind_and_port() {
        let bin = PathBuf::from("/usr/bin/moltis");
        let opts = GatewayServiceOpts {
            bind: Some("0.0.0.0".into()),
            port: Some(9090),
            log_level: "warn".into(),
        };
        let log = PathBuf::from("/tmp/moltis.log");

        let unit = generate_systemd_unit(&bin, &opts, &log);

        assert!(unit.contains("--bind 0.0.0.0"));
        assert!(unit.contains("--port 9090"));
        assert!(unit.contains("--log-level warn"));
    }

    #[test]
    fn systemd_unit_omits_optional_flags() {
        let bin = PathBuf::from("/usr/bin/moltis");
        let opts = GatewayServiceOpts {
            bind: None,
            port: None,
            log_level: "info".into(),
        };
        let log = PathBuf::from("/tmp/moltis.log");

        let unit = generate_systemd_unit(&bin, &opts, &log);

        assert!(!unit.contains("--bind"));
        assert!(!unit.contains("--port"));
    }

    #[test]
    fn detects_container_systemctl_wrapper() {
        assert!(systemd_unavailable_output(
            r#""systemd" is not running in this container due to its overhead."#
        ));
        assert!(systemd_unavailable_output(
            "System has not been booted with systemd as init system"
        ));
        assert!(systemd_unavailable_output(
            "Failed to connect to bus: No medium found"
        ));
        assert!(!systemd_unavailable_output("inactive"));
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("abc"), "'abc'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn parses_linux_proc_stat_state() {
        assert_eq!(linux_proc_stat_state("123 (moltis) S 1 2 3"), Some('S'));
        assert_eq!(
            linux_proc_stat_state("123 (name with spaces) Z 1 2 3"),
            Some('Z')
        );
        assert_eq!(linux_proc_stat_state("invalid"), None);
    }

    #[test]
    fn process_supervisor_script_runs_gateway_with_flags() {
        let bin = PathBuf::from("/opt/moltis bin/moltis");
        let opts = GatewayServiceOpts {
            bind: Some("0.0.0.0".into()),
            port: Some(9090),
            log_level: "debug".into(),
        };
        let log = PathBuf::from("/tmp/moltis service.log");

        let script = generate_process_supervisor_script(&bin, &opts, &log).unwrap();

        assert!(script.contains("MOLTIS_BIN='/opt/moltis bin/moltis'"));
        assert!(script.contains(
            "RUST_LOG=info \"$MOLTIS_BIN\" '--log-level' 'debug' '--bind' '0.0.0.0' '--port' '9090'"
        ));
        assert!(script.contains("LOG_FILE='/tmp/moltis service.log'"));
        assert!(script.contains("RESTART_DELAY=10"));
        assert!(!script.contains("MOLTIS_ARGS="));
    }

    #[test]
    fn status_display() {
        assert_eq!(
            ServiceStatus::Running { pid: Some(42) }.to_string(),
            "running (pid 42)"
        );
        assert_eq!(ServiceStatus::Running { pid: None }.to_string(), "running");
        assert_eq!(ServiceStatus::Stopped.to_string(), "stopped");
        assert_eq!(ServiceStatus::NotInstalled.to_string(), "not installed");
        assert_eq!(
            ServiceStatus::Unknown("failed".into()).to_string(),
            "unknown: failed"
        );
    }
}
