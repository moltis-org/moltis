//! Tailscale Serve/Funnel integration.
//!
//! Shells out to the `tailscale` CLI to manage HTTPS proxying:
//! - **Serve**: exposes the gateway over HTTPS within the tailnet.
//! - **Funnel**: exposes the gateway to the public internet via Tailscale.

use std::net::IpAddr;

use {
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    tracing::{info, warn},
};

// ── Types ────────────────────────────────────────────────────────────────────

/// Tailscale operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TailscaleMode {
    Off,
    Serve,
    Funnel,
}

impl TailscaleMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Serve => "serve",
            Self::Funnel => "funnel",
        }
    }
}

impl std::fmt::Display for TailscaleMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for TailscaleMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "serve" => Ok(Self::Serve),
            "funnel" => Ok(Self::Funnel),
            other => {
                anyhow::bail!("unknown tailscale mode: '{other}' (expected off, serve, or funnel)")
            },
        }
    }
}

/// Current tailscale status.
#[derive(Debug, Clone, Serialize)]
pub struct TailscaleStatus {
    pub mode: TailscaleMode,
    pub hostname: Option<String>,
    pub url: Option<String>,
    pub tailscale_up: bool,
    pub installed: bool,
    pub tailnet: Option<String>,
    pub version: Option<String>,
    pub login_name: Option<String>,
    pub tailscale_ip: Option<String>,
}

// ── Trait ─────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait TailscaleManager: Send + Sync {
    /// Get current tailscale status.
    async fn status(&self) -> anyhow::Result<TailscaleStatus>;
    /// Enable tailscale serve proxying to the given port.
    /// When `tls` is true, uses `https+insecure://` to connect to a local TLS server.
    async fn enable_serve(&self, port: u16, tls: bool) -> anyhow::Result<()>;
    /// Enable tailscale funnel proxying to the given port.
    /// When `tls` is true, uses `https+insecure://` to connect to a local TLS server.
    async fn enable_funnel(&self, port: u16, tls: bool) -> anyhow::Result<()>;
    /// Disable serve/funnel (reset).
    async fn disable(&self) -> anyhow::Result<()>;
    /// Get the tailscale hostname for this machine.
    async fn hostname(&self) -> anyhow::Result<Option<String>>;
}

// ── CLI-based implementation ─────────────────────────────────────────────────

/// Implementation that shells out to the `tailscale` CLI binary.
#[derive(Default)]
pub struct CliTailscaleManager;

impl CliTailscaleManager {
    pub fn new() -> Self {
        Self
    }

    async fn run_command(args: &[&str]) -> anyhow::Result<std::process::Output> {
        Self::run_command_timeout(args, std::time::Duration::from_secs(10)).await
    }

    async fn run_command_timeout(
        args: &[&str],
        timeout: std::time::Duration,
    ) -> anyhow::Result<std::process::Output> {
        use tokio::io::AsyncReadExt;

        let cmd_str = format!("tailscale {}", args.join(" "));
        tracing::debug!(cmd = %cmd_str, "running tailscale command");

        let mut child = tokio::process::Command::new("tailscale")
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to run tailscale CLI: {e}"))?;

        let mut stdout_handle = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to capture tailscale stdout"))?;
        let mut stderr_handle = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to capture tailscale stderr"))?;

        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        let mut stdout_tmp = [0u8; 4096];
        let mut stderr_tmp = [0u8; 4096];

        let deadline = tokio::time::Instant::now() + timeout;

        // Read output incrementally. If the process produces output but doesn't
        // exit within 2s after the last read, treat it as hanging.
        loop {
            tokio::select! {
                status = child.wait() => {
                    // Process exited — drain remaining output.
                    let _ = stdout_handle.read_to_end(&mut stdout_buf).await;
                    let _ = stderr_handle.read_to_end(&mut stderr_buf).await;
                    let status = status.map_err(|e| anyhow::anyhow!("failed to run tailscale CLI: {e}"))?;
                    if !status.success() {
                        let stderr_str = String::from_utf8_lossy(&stderr_buf);
                        tracing::debug!(
                            cmd = %cmd_str,
                            exit_code = ?status.code(),
                            stderr = %stderr_str.trim(),
                            "tailscale command failed"
                        );
                    }
                    return Ok(std::process::Output {
                        status,
                        stdout: stdout_buf,
                        stderr: stderr_buf,
                    });
                }
                n = stdout_handle.read(&mut stdout_tmp) => {
                    match n {
                        Ok(0) => {}, // EOF on stdout
                        Ok(n) => stdout_buf.extend_from_slice(&stdout_tmp[..n]),
                        Err(_) => {},
                    }
                }
                n = stderr_handle.read(&mut stderr_tmp) => {
                    match n {
                        Ok(0) => {}, // EOF on stderr
                        Ok(n) => stderr_buf.extend_from_slice(&stderr_tmp[..n]),
                        Err(_) => {},
                    }
                }
                _ = tokio::time::sleep_until(deadline) => {
                    break; // overall timeout
                }
            }

            // Once we have stdout output (the meaningful content, not just
            // stderr warnings), give a short grace period while still reading
            // pipes, then bail — the process is likely waiting for user input.
            if !stdout_buf.is_empty() {
                let grace = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
                loop {
                    tokio::select! {
                        status = child.wait() => {
                            let _ = stdout_handle.read_to_end(&mut stdout_buf).await;
                            let _ = stderr_handle.read_to_end(&mut stderr_buf).await;
                            let status = status.map_err(|e| anyhow::anyhow!("tailscale CLI: {e}"))?;
                            return Ok(std::process::Output {
                                status,
                                stdout: stdout_buf,
                                stderr: stderr_buf,
                            });
                        }
                        n = stdout_handle.read(&mut stdout_tmp) => {
                            if let Ok(n @ 1..) = n { stdout_buf.extend_from_slice(&stdout_tmp[..n]); }
                        }
                        n = stderr_handle.read(&mut stderr_tmp) => {
                            if let Ok(n @ 1..) = n { stderr_buf.extend_from_slice(&stderr_tmp[..n]); }
                        }
                        _ = tokio::time::sleep_until(grace) => {
                            break;
                        }
                    }
                }
                break; // exit outer loop after grace period
            }
        }

        // Process is hanging — kill and return captured output as error.
        let _ = child.kill().await;
        let _ = child.wait().await;

        let stdout_str = String::from_utf8_lossy(&stdout_buf).trim().to_string();
        let stderr_str = String::from_utf8_lossy(&stderr_buf).trim().to_string();
        warn!(
            cmd = %cmd_str,
            stdout = %stdout_str,
            stderr = %stderr_str,
            "tailscale command timed out"
        );

        let combined = [stdout_str, stderr_str]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        let msg = if combined.is_empty() {
            format!(
                "tailscale command timed out after {}s. \
                 Try running `{cmd_str}` in a terminal to see what's needed.",
                timeout.as_secs(),
            )
        } else {
            combined
        };
        anyhow::bail!("{msg}");
    }

    async fn parse_status_json() -> anyhow::Result<serde_json::Value> {
        let output = Self::run_command(&["status", "--json"]).await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("tailscale status failed: {stderr}");
        }
        let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        Ok(value)
    }
}

#[async_trait]
impl TailscaleManager for CliTailscaleManager {
    async fn status(&self) -> anyhow::Result<TailscaleStatus> {
        // Check if the tailscale CLI is installed.
        let installed = tokio::process::Command::new("tailscale")
            .arg("version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .is_ok();

        if !installed {
            return Ok(TailscaleStatus {
                mode: TailscaleMode::Off,
                hostname: None,
                url: None,
                tailscale_up: false,
                installed: false,
                tailnet: None,
                version: None,
                login_name: None,
                tailscale_ip: None,
            });
        }

        // Parse `tailscale status --json` once to get hostname, tailnet, version, login.
        let (hostname, tailnet, version, login_name, tailscale_ip) =
            match Self::parse_status_json().await {
                Ok(val) => {
                    let dns_name = val
                        .pointer("/Self/DNSName")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim_end_matches('.').to_string());
                    let tailnet = val
                        .pointer("/CurrentTailnet/Name")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let ver = val
                        .get("Version")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    // Find login name for the current user (Self/UserID → User map).
                    let login = val.pointer("/Self/UserID").and_then(|uid| {
                        let uid_str = uid.to_string();
                        val.pointer(&format!("/User/{uid_str}/LoginName"))
                            .and_then(|v| v.as_str())
                            .map(String::from)
                    });
                    // First IPv4 address from TailscaleIPs.
                    let ts_ip = val
                        .pointer("/Self/TailscaleIPs/0")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    (dns_name, tailnet, ver, login, ts_ip)
                },
                Err(_) => (None, None, None, None, None),
            };

        let tailscale_up = hostname.is_some();

        // Check if serve or funnel is active by looking at `tailscale serve status --json`.
        let serve_output = Self::run_command(&["serve", "status", "--json"]).await;
        let mode = match serve_output {
            Ok(ref out) if out.status.success() => {
                let body = String::from_utf8_lossy(&out.stdout);
                let body = body.trim();
                if body.is_empty() || body == "{}" || body == "null" {
                    TailscaleMode::Off
                } else if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
                    if has_funnel_enabled(&val) {
                        TailscaleMode::Funnel
                    } else {
                        TailscaleMode::Serve
                    }
                } else {
                    TailscaleMode::Off
                }
            },
            _ => TailscaleMode::Off,
        };

        let url = hostname.as_ref().map(|h| format!("https://{h}"));

        Ok(TailscaleStatus {
            mode,
            hostname,
            url,
            tailscale_up,
            installed,
            tailnet,
            version,
            login_name,
            tailscale_ip,
        })
    }

    async fn enable_serve(&self, port: u16, tls: bool) -> anyhow::Result<()> {
        // First reset any existing serve/funnel.
        info!("resetting tailscale serve before enabling");
        let _ = Self::run_command(&["serve", "reset"]).await;

        let target = serve_target(port, tls);
        info!(port, target = %target, tls, "running tailscale serve --bg");
        let output = Self::run_command(&["serve", "--bg", "--yes", &target]).await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("tailscale serve failed: {stderr}");
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        info!(port, stdout = %stdout.trim(), "tailscale serve enabled");
        Ok(())
    }

    async fn enable_funnel(&self, port: u16, tls: bool) -> anyhow::Result<()> {
        // First reset any existing serve/funnel.
        info!("resetting tailscale funnel before enabling");
        let _ = Self::run_command(&["funnel", "reset"]).await;

        let target = serve_target(port, tls);
        info!(port, target = %target, tls, "running tailscale funnel --bg");
        let output = Self::run_command(&["funnel", "--bg", "--yes", &target]).await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("tailscale funnel failed: {stderr}");
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        info!(port, stdout = %stdout.trim(), "tailscale funnel enabled");
        Ok(())
    }

    async fn disable(&self) -> anyhow::Result<()> {
        let serve_out = Self::run_command(&["serve", "reset"]).await?;
        if !serve_out.status.success() {
            let stderr = String::from_utf8_lossy(&serve_out.stderr);
            warn!("tailscale serve reset: {stderr}");
        }
        let funnel_out = Self::run_command(&["funnel", "reset"]).await?;
        if !funnel_out.status.success() {
            let stderr = String::from_utf8_lossy(&funnel_out.stderr);
            warn!("tailscale funnel reset: {stderr}");
        }
        info!("tailscale serve/funnel disabled");
        Ok(())
    }

    async fn hostname(&self) -> anyhow::Result<Option<String>> {
        match Self::parse_status_json().await {
            Ok(val) => {
                // The JSON has a "Self" object with "DNSName" field.
                let dns_name = val
                    .pointer("/Self/DNSName")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim_end_matches('.').to_string());
                Ok(dns_name)
            },
            Err(_) => Ok(None),
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Build the target URL for tailscale serve/funnel.
/// When the gateway uses TLS, tailscale needs `https+insecure://` to proxy to a
/// server with a self-signed certificate.
fn serve_target(port: u16, tls: bool) -> String {
    if tls {
        format!("https+insecure://127.0.0.1:{port}")
    } else {
        format!("http://127.0.0.1:{port}")
    }
}

/// Check if the serve status JSON indicates funnel is enabled.
fn has_funnel_enabled(val: &serde_json::Value) -> bool {
    // tailscale serve status --json returns something like:
    // { "TCP": { "443": { "HTTPS": true } }, "AllowFunnel": { "443": true } }
    // or in newer versions, check for AllowFunnel or Funnel keys.
    if let Some(obj) = val.as_object()
        && let Some(af) = obj.get("AllowFunnel")
        && let Some(af_obj) = af.as_object()
    {
        return af_obj.values().any(|v| v.as_bool().unwrap_or(false));
    }
    false
}

/// Returns true if the given address is a loopback address.
pub fn is_loopback_addr(addr: &str) -> bool {
    match addr {
        "localhost" | "127.0.0.1" | "::1" => true,
        other => other.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()),
    }
}

/// Validate tailscale configuration constraints.
///
/// - When tailscale mode is not "off", the gateway must be bound to a loopback address.
/// - When funnel mode is requested, password auth must be set up (setup complete).
pub fn validate_tailscale_config(
    mode: TailscaleMode,
    bind_addr: &str,
    auth_setup_complete: bool,
) -> anyhow::Result<()> {
    if mode == TailscaleMode::Off {
        return Ok(());
    }

    if !is_loopback_addr(bind_addr) {
        anyhow::bail!(
            "tailscale {} requires the gateway to bind to a loopback address (127.0.0.1, ::1, or localhost), but got '{bind_addr}'",
            mode
        );
    }

    if mode == TailscaleMode::Funnel && !auth_setup_complete {
        anyhow::bail!(
            "tailscale funnel requires password authentication to be configured. \
             Set a password first (via setup code or MOLTIS_PASSWORD) before enabling funnel."
        );
    }

    Ok(())
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_from_str() {
        assert_eq!("off".parse::<TailscaleMode>().unwrap(), TailscaleMode::Off);
        assert_eq!(
            "serve".parse::<TailscaleMode>().unwrap(),
            TailscaleMode::Serve
        );
        assert_eq!(
            "funnel".parse::<TailscaleMode>().unwrap(),
            TailscaleMode::Funnel
        );
        assert_eq!(
            "Serve".parse::<TailscaleMode>().unwrap(),
            TailscaleMode::Serve
        );
        assert!("invalid".parse::<TailscaleMode>().is_err());
    }

    #[test]
    fn mode_display() {
        assert_eq!(TailscaleMode::Off.to_string(), "off");
        assert_eq!(TailscaleMode::Serve.to_string(), "serve");
        assert_eq!(TailscaleMode::Funnel.to_string(), "funnel");
    }

    #[test]
    fn loopback_check() {
        assert!(is_loopback_addr("127.0.0.1"));
        assert!(is_loopback_addr("localhost"));
        assert!(is_loopback_addr("::1"));
        assert!(!is_loopback_addr("0.0.0.0"));
        assert!(!is_loopback_addr("192.168.1.1"));
        assert!(!is_loopback_addr("10.0.0.1"));
    }

    #[test]
    fn validate_off_always_ok() {
        assert!(validate_tailscale_config(TailscaleMode::Off, "0.0.0.0", false).is_ok());
    }

    #[test]
    fn validate_serve_requires_loopback() {
        assert!(validate_tailscale_config(TailscaleMode::Serve, "127.0.0.1", false).is_ok());
        assert!(validate_tailscale_config(TailscaleMode::Serve, "0.0.0.0", false).is_err());
    }

    #[test]
    fn validate_funnel_requires_password() {
        assert!(validate_tailscale_config(TailscaleMode::Funnel, "127.0.0.1", true).is_ok());
        assert!(validate_tailscale_config(TailscaleMode::Funnel, "127.0.0.1", false).is_err());
    }

    #[test]
    fn has_funnel_enabled_parsing() {
        let with_funnel: serde_json::Value = serde_json::json!({"AllowFunnel": {"443": true}});
        assert!(has_funnel_enabled(&with_funnel));

        let without_funnel: serde_json::Value =
            serde_json::json!({"TCP": {"443": {"HTTPS": true}}});
        assert!(!has_funnel_enabled(&without_funnel));

        let empty: serde_json::Value = serde_json::json!({});
        assert!(!has_funnel_enabled(&empty));
    }
}
