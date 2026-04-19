//! Install / manage a headless node as an OS service.
//!
//! - **macOS**: launchd user agent (`~/Library/LaunchAgents/org.moltis.node.plist`)
//! - **Linux**: systemd user unit (`~/.config/systemd/user/moltis-node.service`)
//!
//! The service wraps `moltis node run` with the persisted connection parameters.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use {
    serde::{Deserialize, Serialize},
    tracing::{debug, info},
};

use crate::error::{Error, Result};

// ── Persisted connection config ────────────────────────────────────────────

/// Connection parameters saved to `~/.moltis/node.json` so the service can
/// start without CLI flags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub gateway_url: String,
    pub device_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

fn default_timeout() -> u64 {
    300
}

impl ServiceConfig {
    /// Load from `<data_dir>/node.json`.
    pub fn load(data_dir: &Path) -> Result<Self> {
        let path = data_dir.join("node.json");
        let contents = fs::read_to_string(&path)
            .map_err(|e| Error::Config(format!("cannot read {}: {e}", path.display())))?;
        let config: Self = serde_json::from_str(&contents)?;
        Ok(config)
    }

    /// Save to `<data_dir>/node.json`.
    pub fn save(&self, data_dir: &Path) -> Result<()> {
        fs::create_dir_all(data_dir)?;
        let path = data_dir.join("node.json");
        let json = serde_json::to_string_pretty(self)?;
        fs::write(&path, json)?;
        info!(path = %path.display(), "saved node config");
        Ok(())
    }
}

// ── Platform constants ─────────────────────────────────────────────────────

/// macOS launchd label.
const LAUNCHD_LABEL: &str = "org.moltis.node";

/// systemd user unit name.
const SYSTEMD_UNIT: &str = "moltis-node.service";

// ── Service actions ────────────────────────────────────────────────────────

/// Install the node as an OS service.
///
/// Saves the connection config, generates the service file, and enables it.
pub fn install(data_dir: &Path, config: &ServiceConfig) -> Result<()> {
    config.save(data_dir)?;

    let moltis_bin = resolve_binary()?;
    let log_path = data_dir.join("node.log");

    if cfg!(target_os = "macos") {
        install_launchd(&moltis_bin, config, &log_path)
    } else if cfg!(target_os = "linux") {
        install_systemd(&moltis_bin, config, &log_path)
    } else {
        Err(Error::UnsupportedPlatform)
    }
}

/// Uninstall the service and remove generated files.
pub fn uninstall(data_dir: &Path) -> Result<()> {
    if cfg!(target_os = "macos") {
        uninstall_launchd()
    } else if cfg!(target_os = "linux") {
        uninstall_systemd()
    } else {
        Err(Error::UnsupportedPlatform)
    }?;

    // Remove persisted config.
    let config_path = data_dir.join("node.json");
    if config_path.exists() {
        fs::remove_file(&config_path)?;
        info!(path = %config_path.display(), "removed node config");
    }

    Ok(())
}

/// Print the service status.
pub fn status() -> Result<ServiceStatus> {
    if cfg!(target_os = "macos") {
        status_launchd()
    } else if cfg!(target_os = "linux") {
        status_systemd()
    } else {
        Err(Error::UnsupportedPlatform)
    }
}

/// Stop the service.
pub fn stop() -> Result<()> {
    if cfg!(target_os = "macos") {
        stop_launchd()
    } else if cfg!(target_os = "linux") {
        stop_systemd()
    } else {
        Err(Error::UnsupportedPlatform)
    }
}

/// Restart the service.
pub fn restart() -> Result<()> {
    if cfg!(target_os = "macos") {
        restart_launchd()
    } else if cfg!(target_os = "linux") {
        restart_systemd()
    } else {
        Err(Error::UnsupportedPlatform)
    }
}

/// Print the service log path.
pub fn log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("node.log")
}

// ── Status type ────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ServiceStatus {
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
            Self::Unknown(msg) => write!(f, "unknown: {msg}"),
        }
    }
}

// ── Binary resolution ──────────────────────────────────────────────────────

fn resolve_binary() -> Result<PathBuf> {
    // Prefer the running binary if it looks right.
    if let Ok(exe) = std::env::current_exe() {
        let name = exe.file_name().unwrap_or_default().to_string_lossy();
        if name == "moltis" || name.starts_with("moltis-") {
            return Ok(exe);
        }
    }

    // Fall back to PATH lookup.
    which::which("moltis").map_err(|_| {
        Error::Config("cannot find 'moltis' binary; ensure it is installed and in PATH".into())
    })
}

// ── macOS launchd ──────────────────────────────────────────────────────────

fn launchd_plist_path() -> Result<PathBuf> {
    let home = home_dir()?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LAUNCHD_LABEL}.plist")))
}

/// Escape special XML characters for safe interpolation into plist values.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Generate a launchd plist XML string.
pub fn generate_launchd_plist(
    moltis_bin: &Path,
    config: &ServiceConfig,
    log_path: &Path,
) -> String {
    let bin = xml_escape(&moltis_bin.display().to_string());
    let log = xml_escape(&log_path.display().to_string());

    let mut args = vec![
        format!("    <string>{bin}</string>"),
        "    <string>node</string>".to_string(),
        "    <string>run</string>".to_string(),
        format!("    <string>--host</string>"),
        format!("    <string>{}</string>", xml_escape(&config.gateway_url)),
        format!("    <string>--token</string>"),
        format!("    <string>{}</string>", xml_escape(&config.device_token)),
        format!("    <string>--timeout</string>"),
        format!("    <string>{}</string>", config.timeout),
    ];

    if let Some(ref id) = config.node_id {
        args.push("    <string>--node-id</string>".to_string());
        args.push(format!("    <string>{}</string>", xml_escape(id)));
    }
    if let Some(ref name) = config.display_name {
        args.push("    <string>--name</string>".to_string());
        args.push(format!("    <string>{}</string>", xml_escape(name)));
    }
    if let Some(ref dir) = config.working_dir {
        args.push("    <string>--working-dir</string>".to_string());
        args.push(format!("    <string>{}</string>", xml_escape(dir)));
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

fn install_launchd(moltis_bin: &Path, config: &ServiceConfig, log_path: &Path) -> Result<()> {
    let plist_path = launchd_plist_path()?;

    // Unload first if already loaded (ignore errors).
    let _ = Command::new("launchctl")
        .args([
            "bootout",
            &format!("gui/{}", uid()),
            plist_path.to_str().unwrap_or_default(),
        ])
        .output();

    let plist = generate_launchd_plist(moltis_bin, config, log_path);

    if let Some(parent) = plist_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&plist_path, &plist)?;
    info!(path = %plist_path.display(), "wrote launchd plist");

    let output = Command::new("launchctl")
        .args([
            "bootstrap",
            &format!("gui/{}", uid()),
            plist_path.to_str().unwrap_or_default(),
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Service(format!(
            "launchctl bootstrap failed: {stderr}"
        )));
    }

    info!("node service installed and started");
    Ok(())
}

fn uninstall_launchd() -> Result<()> {
    let plist_path = launchd_plist_path()?;

    if !plist_path.exists() {
        return Err(Error::Service(
            "service not installed (plist not found)".into(),
        ));
    }

    let _ = Command::new("launchctl")
        .args([
            "bootout",
            &format!("gui/{}", uid()),
            plist_path.to_str().unwrap_or_default(),
        ])
        .output();

    fs::remove_file(&plist_path)?;
    info!(path = %plist_path.display(), "removed launchd plist");
    info!("node service uninstalled");
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

    // Parse PID from `launchctl print` output: "pid = 12345"
    let pid = stdout.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.starts_with("pid = ") {
            trimmed.strip_prefix("pid = ")?.parse::<u32>().ok()
        } else {
            None
        }
    });

    Ok(ServiceStatus::Running { pid })
}

fn stop_launchd() -> Result<()> {
    let plist_path = launchd_plist_path()?;
    if !plist_path.exists() {
        return Err(Error::Service("service not installed".into()));
    }

    let output = Command::new("launchctl")
        .args(["kill", "SIGTERM", &format!("gui/{}/{LAUNCHD_LABEL}", uid())])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // "No such process" is fine — already stopped.
        if !stderr.contains("No such process") && !stderr.contains("3: No such process") {
            return Err(Error::Service(format!("launchctl kill failed: {stderr}")));
        }
    }

    info!("node service stopped");
    Ok(())
}

fn restart_launchd() -> Result<()> {
    let plist_path = launchd_plist_path()?;
    if !plist_path.exists() {
        return Err(Error::Service("service not installed".into()));
    }

    // kickstart -k kills and restarts the service.
    let output = Command::new("launchctl")
        .args(["kickstart", "-k", &format!("gui/{}/{LAUNCHD_LABEL}", uid())])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Service(format!(
            "launchctl kickstart failed: {stderr}"
        )));
    }

    info!("node service restarted");
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

/// Generate a systemd user unit file.
pub fn generate_systemd_unit(moltis_bin: &Path, config: &ServiceConfig, log_path: &Path) -> String {
    let bin = moltis_bin.display();
    let log = log_path.display();

    let mut exec_args = format!(
        "{bin} node run --host \"{}\" --token \"{}\" --timeout {}",
        config.gateway_url.replace('"', "\\\""),
        config.device_token.replace('"', "\\\""),
        config.timeout,
    );

    if let Some(ref id) = config.node_id {
        exec_args.push_str(&format!(" --node-id \"{}\"", id.replace('"', "\\\"")));
    }
    if let Some(ref name) = config.display_name {
        exec_args.push_str(&format!(" --name \"{}\"", name.replace('"', "\\\"")));
    }
    if let Some(ref dir) = config.working_dir {
        exec_args.push_str(&format!(" --working-dir \"{}\"", dir.replace('"', "\\\"")));
    }

    format!(
        r#"[Unit]
Description=Moltis Node Host
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

fn install_systemd(moltis_bin: &Path, config: &ServiceConfig, log_path: &Path) -> Result<()> {
    let unit_path = systemd_unit_path()?;

    // Stop if already running (ignore errors).
    let _ = Command::new("systemctl")
        .args(["--user", "stop", SYSTEMD_UNIT])
        .output();

    let unit = generate_systemd_unit(moltis_bin, config, log_path);

    if let Some(parent) = unit_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&unit_path, &unit)?;
    info!(path = %unit_path.display(), "wrote systemd unit");

    // Reload, enable, start.
    run_systemctl(&["daemon-reload"])?;
    run_systemctl(&["enable", SYSTEMD_UNIT])?;
    run_systemctl(&["start", SYSTEMD_UNIT])?;

    info!("node service installed and started");
    Ok(())
}

fn uninstall_systemd() -> Result<()> {
    let unit_path = systemd_unit_path()?;

    if !unit_path.exists() {
        return Err(Error::Service(
            "service not installed (unit file not found)".into(),
        ));
    }

    let _ = run_systemctl(&["stop", SYSTEMD_UNIT]);
    let _ = run_systemctl(&["disable", SYSTEMD_UNIT]);

    fs::remove_file(&unit_path)?;
    let _ = run_systemctl(&["daemon-reload"]);

    info!(path = %unit_path.display(), "removed systemd unit");
    info!("node service uninstalled");
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
            // Get PID.
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
        return Err(Error::Service("service not installed".into()));
    }
    run_systemctl(&["stop", SYSTEMD_UNIT])?;
    info!("node service stopped");
    Ok(())
}

fn restart_systemd() -> Result<()> {
    let unit_path = systemd_unit_path()?;
    if !unit_path.exists() {
        return Err(Error::Service("service not installed".into()));
    }
    run_systemctl(&["restart", SYSTEMD_UNIT])?;
    info!("node service restarted");
    Ok(())
}

fn run_systemctl(args: &[&str]) -> Result<()> {
    let mut full_args = vec!["--user"];
    full_args.extend_from_slice(args);

    debug!(args = ?full_args, "systemctl");

    let output = Command::new("systemctl").args(&full_args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Service(format!(
            "systemctl {} failed: {stderr}",
            args.join(" ")
        )));
    }
    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Error::Config("cannot determine home directory (HOME not set)".into()))
}

fn uid() -> u32 {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(501) // macOS default user uid
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn service_config_roundtrip() {
        let config = ServiceConfig {
            gateway_url: "ws://localhost:9090/ws".into(),
            device_token: "tok_abc".into(),
            node_id: Some("my-node".into()),
            display_name: Some("MacBook".into()),
            working_dir: None,
            timeout: 300,
        };

        let json = serde_json::to_string_pretty(&config).unwrap();
        let loaded: ServiceConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.gateway_url, config.gateway_url);
        assert_eq!(loaded.device_token, config.device_token);
        assert_eq!(loaded.node_id, config.node_id);
        assert_eq!(loaded.display_name, config.display_name);
        assert_eq!(loaded.timeout, 300);
    }

    #[test]
    fn service_config_save_and_load() {
        let dir = std::env::temp_dir().join("moltis-service-test");
        let _ = fs::remove_dir_all(&dir);

        let config = ServiceConfig {
            gateway_url: "ws://host:9090/ws".into(),
            device_token: "tok_123".into(),
            node_id: None,
            display_name: None,
            working_dir: Some("/tmp".into()),
            timeout: 600,
        };

        config.save(&dir).unwrap();
        let loaded = ServiceConfig::load(&dir).unwrap();

        assert_eq!(loaded.gateway_url, "ws://host:9090/ws");
        assert_eq!(loaded.device_token, "tok_123");
        assert_eq!(loaded.working_dir.as_deref(), Some("/tmp"));
        assert_eq!(loaded.timeout, 600);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn launchd_plist_contains_required_elements() {
        let bin = PathBuf::from("/usr/local/bin/moltis");
        let config = ServiceConfig {
            gateway_url: "ws://gw:9090/ws".into(),
            device_token: "tok_test".into(),
            node_id: Some("node-42".into()),
            display_name: Some("Test Node".into()),
            working_dir: Some("/home/user".into()),
            timeout: 120,
        };
        let log = PathBuf::from("/tmp/node.log");

        let plist = generate_launchd_plist(&bin, &config, &log);

        assert!(plist.contains("org.moltis.node"));
        assert!(plist.contains("/usr/local/bin/moltis"));
        assert!(plist.contains("ws://gw:9090/ws"));
        assert!(plist.contains("tok_test"));
        assert!(plist.contains("node-42"));
        assert!(plist.contains("Test Node"));
        assert!(plist.contains("/home/user"));
        assert!(plist.contains("120"));
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
        assert!(plist.contains("/tmp/node.log"));
        // Verify it's valid-ish XML.
        assert!(plist.starts_with("<?xml"));
        assert!(plist.contains("</plist>"));
    }

    #[test]
    fn launchd_plist_omits_optional_fields() {
        let bin = PathBuf::from("/usr/local/bin/moltis");
        let config = ServiceConfig {
            gateway_url: "ws://gw:9090/ws".into(),
            device_token: "tok_test".into(),
            node_id: None,
            display_name: None,
            working_dir: None,
            timeout: 300,
        };
        let log = PathBuf::from("/tmp/node.log");

        let plist = generate_launchd_plist(&bin, &config, &log);

        assert!(!plist.contains("--node-id"));
        assert!(!plist.contains("--name"));
        assert!(!plist.contains("--working-dir"));
    }

    #[test]
    fn systemd_unit_contains_required_elements() {
        let bin = PathBuf::from("/usr/bin/moltis");
        let config = ServiceConfig {
            gateway_url: "ws://gw:9090/ws".into(),
            device_token: "tok_sys".into(),
            node_id: Some("sys-node".into()),
            display_name: Some("Server".into()),
            working_dir: Some("/srv".into()),
            timeout: 600,
        };
        let log = PathBuf::from("/var/log/moltis/node.log");

        let unit = generate_systemd_unit(&bin, &config, &log);

        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Service]"));
        assert!(unit.contains("[Install]"));
        assert!(unit.contains("network-online.target"));
        assert!(unit.contains("/usr/bin/moltis node run"));
        assert!(unit.contains("--host \"ws://gw:9090/ws\""));
        assert!(unit.contains("--token \"tok_sys\""));
        assert!(unit.contains("--node-id \"sys-node\""));
        assert!(unit.contains("--name \"Server\""));
        assert!(unit.contains("--working-dir \"/srv\""));
        assert!(unit.contains("--timeout 600"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("RestartSec=10"));
        assert!(unit.contains("/var/log/moltis/node.log"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn systemd_unit_omits_optional_fields() {
        let bin = PathBuf::from("/usr/bin/moltis");
        let config = ServiceConfig {
            gateway_url: "ws://gw:9090/ws".into(),
            device_token: "tok_min".into(),
            node_id: None,
            display_name: None,
            working_dir: None,
            timeout: 300,
        };
        let log = PathBuf::from("/tmp/node.log");

        let unit = generate_systemd_unit(&bin, &config, &log);

        assert!(!unit.contains("--node-id"));
        assert!(!unit.contains("--name"));
        assert!(!unit.contains("--working-dir"));
    }

    #[test]
    fn service_config_default_timeout() {
        let json = r#"{"gateway_url":"ws://h/ws","device_token":"t"}"#;
        let config: ServiceConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.timeout, 300);
    }

    #[test]
    fn status_display() {
        assert_eq!(
            ServiceStatus::Running { pid: Some(123) }.to_string(),
            "running (pid 123)"
        );
        assert_eq!(ServiceStatus::Running { pid: None }.to_string(), "running");
        assert_eq!(ServiceStatus::Stopped.to_string(), "stopped");
        assert_eq!(ServiceStatus::NotInstalled.to_string(), "not installed");
    }
}
