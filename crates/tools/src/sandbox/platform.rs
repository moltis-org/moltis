//! Lightweight platform-specific sandbox backends.

use async_trait::async_trait;
#[cfg(target_os = "linux")]
use tracing::debug;

use {
    super::types::{Sandbox, SandboxConfig, SandboxId, truncate_output_for_display},
    crate::{
        error::{Error, Result},
        exec::{ExecOpts, ExecResult},
    },
};

/// Cgroup v2 sandbox using `systemd-run --user --scope` (Linux only, no root required).
#[cfg(target_os = "linux")]
pub struct CgroupSandbox {
    pub config: SandboxConfig,
}

#[cfg(target_os = "linux")]
impl CgroupSandbox {
    pub fn new(config: SandboxConfig) -> Self {
        Self { config }
    }

    fn scope_name(&self, id: &SandboxId) -> String {
        let prefix = self
            .config
            .container_prefix
            .as_deref()
            .unwrap_or("moltis-sandbox");
        format!("{}-{}", prefix, id.key)
    }

    fn property_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        let limits = &self.config.resource_limits;
        if let Some(ref mem) = limits.memory_limit {
            args.extend(["--property".to_string(), format!("MemoryMax={mem}")]);
        }
        if let Some(cpu) = limits.cpu_quota {
            let pct = (cpu * 100.0) as u64;
            args.extend(["--property".to_string(), format!("CPUQuota={pct}%")]);
        }
        if let Some(pids) = limits.pids_max {
            args.extend(["--property".to_string(), format!("TasksMax={pids}")]);
        }
        args
    }
}

#[cfg(target_os = "linux")]
#[async_trait]
impl Sandbox for CgroupSandbox {
    fn backend_name(&self) -> &'static str {
        "cgroup"
    }

    async fn ensure_ready(&self, _id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
        let output = tokio::process::Command::new("systemd-run")
            .arg("--version")
            .output()
            .await;
        match output {
            Ok(o) if o.status.success() => {
                debug!("systemd-run available");
                Ok(())
            },
            _ => {
                return Err(Error::message(
                    "systemd-run not found; cgroup sandbox requires systemd",
                ));
            },
        }
    }

    async fn exec(&self, id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        let scope = self.scope_name(id);

        let mut args = vec![
            "--user".to_string(),
            "--scope".to_string(),
            "--unit".to_string(),
            scope,
        ];
        args.extend(self.property_args());
        args.extend(["sh".to_string(), "-c".to_string(), command.to_string()]);

        let mut cmd = tokio::process::Command::new("systemd-run");
        cmd.args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null());

        if let Some(ref dir) = opts.working_dir {
            cmd.current_dir(dir);
        }
        for (k, v) in &opts.env {
            cmd.env(k, v);
        }

        let child = cmd.spawn()?;
        let result = tokio::time::timeout(opts.timeout, child.wait_with_output()).await;

        match result {
            Ok(Ok(output)) => {
                let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();

                truncate_output_for_display(&mut stdout, opts.max_output_bytes);
                truncate_output_for_display(&mut stderr, opts.max_output_bytes);

                Ok(ExecResult {
                    stdout,
                    stderr,
                    exit_code: output.status.code().unwrap_or(-1),
                })
            },
            Ok(Err(e)) => {
                return Err(Error::message(format!("systemd-run exec failed: {e}")));
            },
            Err(_) => {
                return Err(Error::message(format!(
                    "systemd-run exec timed out after {}s",
                    opts.timeout.as_secs()
                )));
            },
        }
    }

    async fn cleanup(&self, id: &SandboxId) -> Result<()> {
        let scope = self.scope_name(id);
        let _ = tokio::process::Command::new("systemctl")
            .args(["--user", "stop", &format!("{scope}.scope")])
            .output()
            .await;
        Ok(())
    }
}

/// Restricted host sandbox providing OS-level isolation (env clearing,
/// restricted PATH, rlimits) without containers or WASM. Commands run on the
/// host via `sh -c` with sanitised environment and ulimit wrappers.
pub struct RestrictedHostSandbox {
    config: SandboxConfig,
}

impl RestrictedHostSandbox {
    pub fn new(config: SandboxConfig) -> Self {
        Self { config }
    }

    /// Wrap a command with shell `ulimit` calls for resource isolation.
    fn build_ulimit_wrapped_command(&self, command: &str) -> String {
        let limits = &self.config.resource_limits;
        let mut preamble = Vec::new();

        // Max user processes.
        let nproc = limits.pids_max.map(u64::from).unwrap_or(256);
        preamble.push(format!("ulimit -u {nproc} 2>/dev/null"));

        // Max open file descriptors.
        preamble.push("ulimit -n 1024 2>/dev/null".to_string());

        // CPU time in seconds.
        let cpu_secs = limits
            .cpu_quota
            .map(|q| q.ceil() as u64 * 60)
            .unwrap_or(300);
        preamble.push(format!("ulimit -t {cpu_secs} 2>/dev/null"));

        // Virtual memory (in KB for ulimit -v).
        let mem_bytes = limits
            .memory_limit
            .as_deref()
            .and_then(parse_memory_limit)
            .unwrap_or(512 * 1024 * 1024);
        let mem_kb = mem_bytes / 1024;
        preamble.push(format!("ulimit -v {mem_kb} 2>/dev/null"));

        format!("{}; {command}", preamble.join("; "))
    }
}

/// Parse a human-readable memory limit like "512M" or "1G" into bytes.
pub(crate) fn parse_memory_limit(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num_part, multiplier) =
        if let Some(n) = s.strip_suffix('G').or_else(|| s.strip_suffix('g')) {
            (n, 1024 * 1024 * 1024)
        } else if let Some(n) = s.strip_suffix('M').or_else(|| s.strip_suffix('m')) {
            (n, 1024 * 1024)
        } else if let Some(n) = s.strip_suffix('K').or_else(|| s.strip_suffix('k')) {
            (n, 1024)
        } else {
            (s, 1)
        };
    num_part.trim().parse::<u64>().ok().map(|n| n * multiplier)
}

#[async_trait]
impl Sandbox for RestrictedHostSandbox {
    fn backend_name(&self) -> &'static str {
        "restricted-host"
    }

    fn is_real(&self) -> bool {
        true
    }

    async fn ensure_ready(&self, _id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
        Ok(())
    }

    async fn exec(&self, _id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        // Wrap the command with shell ulimit calls for resource isolation.
        let wrapped = self.build_ulimit_wrapped_command(command);

        let mut cmd = tokio::process::Command::new("sh");
        cmd.args(["-c", &wrapped]);

        // Scrub all inherited env vars for isolation.
        cmd.env_clear();

        // Set minimal safe environment.
        cmd.env("PATH", "/usr/local/bin:/usr/bin:/bin");
        cmd.env("HOME", "/tmp");
        cmd.env("LANG", "C.UTF-8");

        // Apply user-specified env vars.
        for (k, v) in &opts.env {
            cmd.env(k, v);
        }

        if let Some(ref dir) = opts.working_dir {
            cmd.current_dir(dir);
        }

        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null());

        let child = cmd.spawn()?;
        let result = tokio::time::timeout(opts.timeout, child.wait_with_output()).await;

        match result {
            Ok(Ok(output)) => {
                let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();

                truncate_output_for_display(&mut stdout, opts.max_output_bytes);
                truncate_output_for_display(&mut stderr, opts.max_output_bytes);

                Ok(ExecResult {
                    stdout,
                    stderr,
                    exit_code: output.status.code().unwrap_or(-1),
                })
            },
            Ok(Err(e)) => {
                return Err(Error::message(format!(
                    "restricted-host sandbox exec failed: {e}"
                )));
            },
            Err(_) => {
                return Err(Error::message(format!(
                    "restricted-host sandbox exec timed out after {}s",
                    opts.timeout.as_secs()
                )));
            },
        }
    }

    async fn cleanup(&self, _id: &SandboxId) -> Result<()> {
        Ok(())
    }
}

/// Returns `true` when the WASM sandbox feature is compiled in.
#[cfg(feature = "wasm")]
pub fn is_wasm_sandbox_available() -> bool {
    true
}

#[cfg(not(feature = "wasm"))]
pub fn is_wasm_sandbox_available() -> bool {
    false
}
