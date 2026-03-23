//! `moltis service` subcommands — install/manage the gateway as an OS service.
//!
//! - **macOS**: launchd user agent (`~/Library/LaunchAgents/org.moltis.plist`)
//! - **Linux**: systemd user unit (`~/.config/systemd/user/moltis.service`)

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

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
                install_systemd(&moltis_bin, &opts, &log_path)?;
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
                uninstall_systemd()?;
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
                status_systemd()?
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
                stop_systemd()?;
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
                restart_systemd()?;
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
}

impl std::fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running { pid: Some(p) } => write!(f, "running (pid {p})"),
            Self::Running { pid: None } => write!(f, "running"),
            Self::Stopped => write!(f, "stopped"),
            Self::NotInstalled => write!(f, "not installed"),
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
        format!("    <string>--log-level</string>"),
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

    let state = String::from_utf8_lossy(&output.stdout).trim().to_string();

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
        _ => Ok(ServiceStatus::Stopped),
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
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("systemctl {} failed: {stderr}", args.join(" "));
    }
    Ok(())
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
    fn status_display() {
        assert_eq!(
            ServiceStatus::Running { pid: Some(42) }.to_string(),
            "running (pid 42)"
        );
        assert_eq!(ServiceStatus::Running { pid: None }.to_string(), "running");
        assert_eq!(ServiceStatus::Stopped.to_string(), "stopped");
        assert_eq!(ServiceStatus::NotInstalled.to_string(), "not installed");
    }
}
