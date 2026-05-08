//! Local Firecracker sandbox backend — microVM-based isolation without Docker.
//!
//! Boots ephemeral Firecracker microVMs for sandboxed command execution.
//! Each session gets its own VM with a copy-on-write rootfs, dedicated
//! TAP device, and SSH access for command execution.
//!
//! **Requirements:**
//! - Linux only (Firecracker is Linux-exclusive)
//! - `firecracker` binary installed
//! - Uncompressed Linux kernel (`vmlinux`)
//! - ext4 rootfs image with SSH server and `sandbox` user
//! - Root or `CAP_NET_ADMIN` for TAP device creation

use std::{
    collections::HashMap,
    io::Write as _,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use {
    async_trait::async_trait,
    serde_json::json,
    tokio::{
        io::AsyncReadExt,
        sync::{RwLock, Semaphore},
    },
    tracing::{debug, info, warn},
};

use crate::{
    error::{Error, Result},
    exec::{ExecOpts, ExecResult},
    sandbox::types::{Sandbox, SandboxConfig, SandboxId},
};

const GUEST_USER: &str = "sandbox";
const SUBNET_BASE: &str = "172.16";
const FC_WORKSPACE: &str = "/home/sandbox";
const EXIT_SYMLINK: i32 = 14;
const EXIT_PARENT_MISSING: i32 = 20;

struct FirecrackerVm {
    process: tokio::process::Child,
    api_socket: PathBuf,
    tap_device: String,
    guest_ip: String,
    rootfs_copy: PathBuf,
}

/// Firecracker backend configuration.
#[derive(Debug, Clone)]
pub struct FirecrackerSandboxConfig {
    pub firecracker_bin: PathBuf,
    pub kernel_path: PathBuf,
    pub rootfs_path: PathBuf,
    pub ssh_key_path: PathBuf,
    pub vcpus: u32,
    pub memory_mb: u32,
}

impl Default for FirecrackerSandboxConfig {
    fn default() -> Self {
        Self {
            firecracker_bin: PathBuf::from("firecracker"),
            kernel_path: PathBuf::from("/opt/moltis/vmlinux"),
            rootfs_path: PathBuf::from("/opt/moltis/rootfs.ext4"),
            ssh_key_path: PathBuf::from("/opt/moltis/ssh_key"),
            vcpus: 2,
            memory_mb: 512,
        }
    }
}

pub fn resolve_firecracker_bin(configured: Option<&Path>) -> PathBuf {
    configured.map_or_else(
        || which::which("firecracker").unwrap_or_else(|_| PathBuf::from("firecracker")),
        Path::to_path_buf,
    )
}

pub fn firecracker_bin_available(configured: Option<&Path>) -> bool {
    configured.map_or_else(
        || which::which("firecracker").is_ok(),
        |path| {
            if path.components().count() == 1 {
                which::which(path).is_ok()
            } else {
                path.exists()
            }
        },
    )
}

/// Firecracker sandbox backend.
pub struct FirecrackerSandbox {
    #[allow(dead_code)]
    config: SandboxConfig,
    fc: FirecrackerSandboxConfig,
    active: RwLock<HashMap<String, FirecrackerVm>>,
    creation_permits: RwLock<HashMap<String, Arc<Semaphore>>>,
    subnet_counter: std::sync::atomic::AtomicU16,
}

impl FirecrackerSandbox {
    /// Maximum number of concurrent /30 subnets (256 * 64 = 16384).
    const MAX_SUBNETS: u16 = 256 * 64;

    pub fn new(config: SandboxConfig, fc: FirecrackerSandboxConfig) -> Self {
        Self {
            config,
            fc,
            active: RwLock::new(HashMap::new()),
            creation_permits: RwLock::new(HashMap::new()),
            subnet_counter: std::sync::atomic::AtomicU16::new(1),
        }
    }

    async fn creation_permit(&self, id: &SandboxId) -> Arc<Semaphore> {
        if let Some(permit) = self.creation_permits.read().await.get(&id.key).cloned() {
            return permit;
        }
        let mut permits = self.creation_permits.write().await;
        permits
            .entry(id.key.clone())
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone()
    }

    async fn existing_creation_permit(&self, id: &SandboxId) -> Option<Arc<Semaphore>> {
        self.creation_permits.read().await.get(&id.key).cloned()
    }

    fn allocate_subnet(&self) -> Result<(String, String, u16)> {
        let idx = self
            .subnet_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if idx >= Self::MAX_SUBNETS {
            return Err(Error::message(format!(
                "firecracker: subnet pool exhausted ({} VMs created without cleanup)",
                Self::MAX_SUBNETS
            )));
        }
        let third = idx / 64;
        let fourth_base = (idx % 64) * 4;
        let host_ip = format!("{SUBNET_BASE}.{third}.{}", fourth_base + 1);
        let guest_ip = format!("{SUBNET_BASE}.{third}.{}", fourth_base + 2);
        Ok((host_ip, guest_ip, idx))
    }

    async fn create_tap(tap_name: &str, host_ip: &str) -> Result<()> {
        let status = tokio::process::Command::new("ip")
            .args(["tuntap", "add", "dev", tap_name, "mode", "tap"])
            .status()
            .await
            .map_err(|e| Error::message(format!("firecracker: failed to create TAP: {e}")))?;
        if !status.success() {
            return Err(Error::message(
                "firecracker: ip tuntap add failed (requires root or CAP_NET_ADMIN)",
            ));
        }

        let cidr = format!("{host_ip}/30");
        let _ = tokio::process::Command::new("ip")
            .args(["addr", "add", &cidr, "dev", tap_name])
            .status()
            .await;
        let _ = tokio::process::Command::new("ip")
            .args(["link", "set", tap_name, "up"])
            .status()
            .await;

        Ok(())
    }

    async fn remove_tap(tap_name: &str) {
        let _ = tokio::process::Command::new("ip")
            .args(["link", "del", tap_name])
            .status()
            .await;
    }

    async fn copy_rootfs(base: &Path, dest: &Path) -> Result<()> {
        let status = tokio::process::Command::new("cp")
            .args(["--reflink=auto", "--sparse=auto"])
            .arg(base)
            .arg(dest)
            .status()
            .await
            .map_err(|e| Error::message(format!("firecracker: rootfs copy failed: {e}")))?;
        if !status.success() {
            return Err(Error::message("firecracker: rootfs copy failed"));
        }
        Ok(())
    }

    /// Make an API call to the Firecracker process via curl over Unix socket.
    async fn fc_api_call(
        api_socket: &Path,
        method: &str,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<()> {
        let body_str = serde_json::to_string(body)
            .map_err(|e| Error::message(format!("firecracker: JSON serialize failed: {e}")))?;

        let output = tokio::process::Command::new("curl")
            .args([
                "--unix-socket",
                &api_socket.display().to_string(),
                "-s",
                "-X",
                method,
                &format!("http://localhost{path}"),
                "-H",
                "Content-Type: application/json",
                "-d",
                &body_str,
            ])
            .output()
            .await
            .map_err(|e| Error::message(format!("firecracker: curl failed: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::message(format!(
                "firecracker: API call {method} {path} failed: {stderr}"
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        if serde_json::from_str::<serde_json::Value>(&stdout)
            .is_ok_and(|resp| resp.get("fault_message").is_some())
        {
            return Err(Error::message(format!(
                "firecracker: API error on {method} {path}: {stdout}"
            )));
        }

        Ok(())
    }

    async fn boot_vm(
        &self,
        api_socket: &Path,
        rootfs_path: &Path,
        tap_name: &str,
        guest_ip: &str,
        host_ip: &str,
    ) -> Result<tokio::process::Child> {
        let child = tokio::process::Command::new(&self.fc.firecracker_bin)
            .arg("--api-sock")
            .arg(api_socket)
            .arg("--level")
            .arg("Warning")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::message(format!("firecracker: failed to spawn: {e}")))?;

        // Wait for API socket.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !api_socket.exists() {
            if tokio::time::Instant::now() >= deadline {
                let mut child = child;
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(Error::message(
                    "firecracker: API socket did not appear within 5s",
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // Configure and start the VM. On any error, kill the process to
        // avoid leaving an orphaned Firecracker instance.
        let configure_result = async {
            // Linux kernel ip= format: ip=<client>:<server>:<gw>:<netmask>:<hostname>:<iface>:<autoconf>
            let boot_args = format!(
                "console=ttyS0 reboot=k panic=1 pci=off ip={guest_ip}::{host_ip}:255.255.255.252::eth0:off"
            );
            Self::fc_api_call(
                api_socket,
                "PUT",
                "/boot-source",
                &serde_json::json!({
                    "kernel_image_path": self.fc.kernel_path.display().to_string(),
                    "boot_args": boot_args,
                }),
            )
            .await?;

            Self::fc_api_call(
                api_socket,
                "PUT",
                "/drives/rootfs",
                &serde_json::json!({
                    "drive_id": "rootfs",
                    "path_on_host": rootfs_path.display().to_string(),
                    "is_root_device": true,
                    "is_read_only": false,
                }),
            )
            .await?;

            Self::fc_api_call(
                api_socket,
                "PUT",
                "/machine-config",
                &serde_json::json!({
                    "vcpu_count": self.fc.vcpus,
                    "mem_size_mib": self.fc.memory_mb,
                }),
            )
            .await?;

            Self::fc_api_call(
                api_socket,
                "PUT",
                "/network-interfaces/eth0",
                &serde_json::json!({
                    "iface_id": "eth0",
                    "host_dev_name": tap_name,
                }),
            )
            .await?;

            Self::fc_api_call(
                api_socket,
                "PUT",
                "/actions",
                &serde_json::json!({ "action_type": "InstanceStart" }),
            )
            .await
        }
        .await;

        match configure_result {
            Ok(()) => Ok(child),
            Err(e) => {
                let mut child = child;
                let _ = child.kill().await;
                let _ = child.wait().await;
                Err(e)
            },
        }
    }

    async fn stop_build_vm(process: &mut tokio::process::Child, api_socket: &Path, tap_name: &str) {
        let _ = Self::fc_api_call(
            api_socket,
            "PUT",
            "/actions",
            &serde_json::json!({ "action_type": "SendCtrlAltDel" }),
        )
        .await;
        tokio::time::sleep(Duration::from_secs(3)).await;
        let _ = process.kill().await;
        let _ = process.wait().await;
        Self::remove_tap(tap_name).await;
        let _ = std::fs::remove_file(api_socket);
    }

    async fn cleanup_failed_build_vm(
        process: &mut tokio::process::Child,
        api_socket: &Path,
        tap_name: &str,
        temp_rootfs: &Path,
    ) {
        Self::stop_build_vm(process, api_socket, tap_name).await;
        let _ = std::fs::remove_file(temp_rootfs);
    }

    async fn wait_for_ssh(guest_ip: &str, ssh_key: &Path) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let result = tokio::process::Command::new("ssh")
                .args([
                    "-i",
                    &ssh_key.display().to_string(),
                    "-o",
                    "StrictHostKeyChecking=no",
                    "-o",
                    "UserKnownHostsFile=/dev/null",
                    "-o",
                    "ConnectTimeout=2",
                    "-o",
                    "BatchMode=yes",
                    &format!("{GUEST_USER}@{guest_ip}"),
                    "echo",
                    "ready",
                ])
                .output()
                .await;

            if result.is_ok_and(|output| output.status.success()) {
                return Ok(());
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(Error::message(
                    "firecracker: SSH did not become available within 30s",
                ));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    fn validate_env_key(key: &str) -> Result<()> {
        let mut chars = key.chars();
        let Some(first) = chars.next() else {
            return Err(Error::message(
                "firecracker: empty environment variable name",
            ));
        };
        if !(first == '_' || first.is_ascii_alphabetic()) {
            return Err(Error::message(format!(
                "firecracker: invalid environment variable name '{key}'"
            )));
        }
        if chars.any(|ch| !(ch == '_' || ch.is_ascii_alphanumeric())) {
            return Err(Error::message(format!(
                "firecracker: invalid environment variable name '{key}'"
            )));
        }
        Ok(())
    }

    fn remote_shell_command(cwd: &str, command: &str, env: &[(String, String)]) -> Result<String> {
        let inner = format!("cd {} && {command}", shell_words::quote(cwd));
        let mut words = Vec::with_capacity(env.len() + 3);
        words.push("env".to_string());
        for (key, value) in env {
            Self::validate_env_key(key)?;
            words.push(format!("{key}={value}"));
        }
        words.push("sh".to_string());
        words.push("-lc".to_string());
        words.push(inner);
        Ok(shell_words::join(words))
    }

    async fn collect_ssh_pipe(
        name: &str,
        task: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
    ) -> Result<Vec<u8>> {
        task.await
            .map_err(|e| Error::message(format!("firecracker: SSH {name} reader failed: {e}")))?
            .map_err(|e| Error::message(format!("firecracker: SSH {name} read failed: {e}")))
    }

    async fn ssh_run(
        guest_ip: &str,
        ssh_key: &Path,
        command: &str,
        opts: &ExecOpts,
    ) -> Result<ExecResult> {
        let cwd = opts
            .working_dir
            .as_ref()
            .and_then(|p| p.to_str())
            .unwrap_or(FC_WORKSPACE);

        let remote_command = Self::remote_shell_command(cwd, command, &opts.env)?;
        let ssh_key = ssh_key.display().to_string();
        let destination = format!("{GUEST_USER}@{guest_ip}");

        let mut child = tokio::process::Command::new("ssh")
            .args([
                "-i",
                &ssh_key,
                "-o",
                "StrictHostKeyChecking=no",
                "-o",
                "UserKnownHostsFile=/dev/null",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=5",
                &destination,
                &remote_command,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::message(format!("firecracker: SSH run failed: {e}")))?;

        let Some(mut stdout_pipe) = child.stdout.take() else {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(Error::message("firecracker: SSH stdout pipe unavailable"));
        };
        let Some(mut stderr_pipe) = child.stderr.take() else {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(Error::message("firecracker: SSH stderr pipe unavailable"));
        };

        let stdout_task = tokio::spawn(async move {
            let mut bytes = Vec::new();
            stdout_pipe.read_to_end(&mut bytes).await.map(|_| bytes)
        });
        let stderr_task = tokio::spawn(async move {
            let mut bytes = Vec::new();
            stderr_pipe.read_to_end(&mut bytes).await.map(|_| bytes)
        });

        let status = match tokio::time::timeout(opts.timeout, child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                stdout_task.abort();
                stderr_task.abort();
                return Err(Error::message(format!("firecracker: SSH wait failed: {e}")));
            },
            Err(_) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                stdout_task.abort();
                stderr_task.abort();
                return Err(Error::message(format!(
                    "firecracker: SSH command timed out after {}s",
                    opts.timeout.as_secs()
                )));
            },
        };

        let stdout_bytes = Self::collect_ssh_pipe("stdout", stdout_task).await?;
        let stderr_bytes = Self::collect_ssh_pipe("stderr", stderr_task).await?;
        let mut stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
        let mut stderr = String::from_utf8_lossy(&stderr_bytes).to_string();
        stdout.truncate(stdout.floor_char_boundary(opts.max_output_bytes));
        stderr.truncate(stderr.floor_char_boundary(opts.max_output_bytes));

        Ok(ExecResult {
            stdout,
            stderr,
            exit_code: status.code().unwrap_or(-1),
        })
    }

    async fn session_vm(&self, id: &SandboxId) -> Option<(String, PathBuf)> {
        self.active
            .read()
            .await
            .get(&id.key)
            .map(|vm| (vm.guest_ip.clone(), self.fc.ssh_key_path.clone()))
    }

    fn scp_target(guest_ip: &str, file_path: &str) -> String {
        format!("{GUEST_USER}@{guest_ip}:{}", shell_words::quote(file_path))
    }

    async fn scp_upload(
        guest_ip: &str,
        ssh_key: &Path,
        local_path: &Path,
        remote_path: &str,
    ) -> Result<()> {
        let mut child = tokio::process::Command::new("scp")
            .args([
                "-i",
                &ssh_key.display().to_string(),
                "-o",
                "StrictHostKeyChecking=no",
                "-o",
                "UserKnownHostsFile=/dev/null",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "BatchMode=yes",
                "-q",
            ])
            .arg(local_path)
            .arg(Self::scp_target(guest_ip, remote_path))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::message(format!("firecracker: scp upload failed: {e}")))?;

        let Some(mut stdout_pipe) = child.stdout.take() else {
            let _ = child.start_kill();
            return Err(Error::message("firecracker: scp stdout pipe unavailable"));
        };
        let Some(mut stderr_pipe) = child.stderr.take() else {
            let _ = child.start_kill();
            return Err(Error::message("firecracker: scp stderr pipe unavailable"));
        };

        let stdout_task = tokio::spawn(async move {
            let mut bytes = Vec::new();
            stdout_pipe.read_to_end(&mut bytes).await.map(|_| bytes)
        });
        let stderr_task = tokio::spawn(async move {
            let mut bytes = Vec::new();
            stderr_pipe.read_to_end(&mut bytes).await.map(|_| bytes)
        });

        let status = match tokio::time::timeout(Duration::from_secs(300), child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                stdout_task.abort();
                stderr_task.abort();
                return Err(Error::message(format!("firecracker: scp wait failed: {e}")));
            },
            Err(_) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                stdout_task.abort();
                stderr_task.abort();
                return Err(Error::message("firecracker: scp upload timed out"));
            },
        };

        let _ = Self::collect_ssh_pipe("scp stdout", stdout_task).await?;
        let stderr_bytes = Self::collect_ssh_pipe("scp stderr", stderr_task).await?;
        if !status.success() {
            let stderr = String::from_utf8_lossy(&stderr_bytes);
            return Err(Error::message(format!(
                "firecracker: scp upload failed: {}",
                stderr.trim()
            )));
        }

        Ok(())
    }

    fn write_file_denied(file_path: &str) -> serde_json::Value {
        json!({
            "kind": "path_denied",
            "file_path": file_path,
            "error": "target is a symbolic link; refusing to follow",
            "detail": "firecracker Write rejects symlinks",
        })
    }
}

#[async_trait]
impl Sandbox for FirecrackerSandbox {
    fn backend_name(&self) -> &'static str {
        "firecracker"
    }

    fn is_real(&self) -> bool {
        true
    }

    fn provides_fs_isolation(&self) -> bool {
        true
    }

    fn is_isolated(&self) -> bool {
        true
    }

    /// Build a pre-provisioned rootfs with packages baked in.
    ///
    /// Boots a temporary VM from the base rootfs, installs packages via
    /// apt-get, shuts down, and saves the rootfs as the "built image".
    /// Future `ensure_ready()` calls copy from this pre-built rootfs
    /// instead of the bare one, avoiding per-session package installation.
    async fn build_image(
        &self,
        _base: &str,
        packages: &[String],
    ) -> Result<Option<super::types::BuildImageResult>> {
        use sha2::{Digest, Sha256};

        if packages.is_empty() {
            return Ok(None);
        }

        // Deterministic tag from package list (same as Docker image builder).
        let mut hasher = Sha256::new();
        for pkg in packages {
            hasher.update(pkg.as_bytes());
            hasher.update(b"\n");
        }
        let hash = format!("{:x}", hasher.finalize());
        let tag = format!("moltis-fc-{}", &hash[..12]);

        let data_dir = moltis_config::data_dir();
        let images_dir = data_dir.join("sandbox").join("firecracker").join("images");
        let image_path = images_dir.join(format!("{tag}.ext4"));

        // Check if image already exists (cache hit).
        if image_path.exists() {
            info!(
                tag,
                "firecracker: pre-built rootfs already exists (cache hit)"
            );
            return Ok(Some(super::types::BuildImageResult {
                tag: image_path.display().to_string(),
                built: false,
            }));
        }

        info!(
            tag,
            packages = packages.len(),
            "firecracker: building pre-provisioned rootfs"
        );

        std::fs::create_dir_all(&images_dir).map_err(|e| {
            Error::message(format!("firecracker: failed to create images dir: {e}"))
        })?;

        let temp_rootfs = images_dir.join(format!("{tag}.building.ext4"));
        Self::copy_rootfs(&self.fc.rootfs_path, &temp_rootfs).await?;

        let (host_ip, guest_ip, subnet_idx) = self.allocate_subnet()?;
        let tap_name = format!("moltis-fc{subnet_idx}");
        let api_socket = images_dir.join(format!("{tag}.sock"));
        let _ = std::fs::remove_file(&api_socket);

        if let Err(e) = Self::create_tap(&tap_name, &host_ip).await {
            let _ = std::fs::remove_file(&temp_rootfs);
            return Err(e);
        }

        let mut process = match self
            .boot_vm(&api_socket, &temp_rootfs, &tap_name, &guest_ip, &host_ip)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                Self::remove_tap(&tap_name).await;
                let _ = std::fs::remove_file(&temp_rootfs);
                return Err(e);
            },
        };

        if let Err(e) = Self::wait_for_ssh(&guest_ip, &self.fc.ssh_key_path).await {
            Self::cleanup_failed_build_vm(&mut process, &api_socket, &tap_name, &temp_rootfs).await;
            return Err(e);
        }

        // Install packages.
        let pkg_list = packages.join(" ");
        let install_cmd = format!(
            "apt-get update -qq && apt-get install -y -qq --no-install-recommends {pkg_list}"
        );
        let opts = ExecOpts {
            timeout: Duration::from_secs(600),
            ..Default::default()
        };
        let result = match Self::ssh_run(&guest_ip, &self.fc.ssh_key_path, &install_cmd, &opts)
            .await
        {
            Ok(result) => result,
            Err(e) => {
                Self::cleanup_failed_build_vm(&mut process, &api_socket, &tap_name, &temp_rootfs)
                    .await;
                return Err(e);
            },
        };
        if result.exit_code != 0 {
            warn!(
                tag,
                exit_code = result.exit_code,
                "firecracker: package install during image build failed (continuing)"
            );
        }

        Self::stop_build_vm(&mut process, &api_socket, &tap_name).await;

        // Rename to final path (atomic on same filesystem).
        std::fs::rename(&temp_rootfs, &image_path)
            .map_err(|e| Error::message(format!("firecracker: failed to finalize image: {e}")))?;

        info!(tag, path = %image_path.display(), "firecracker: pre-built rootfs ready");

        Ok(Some(super::types::BuildImageResult {
            tag: image_path.display().to_string(),
            built: true,
        }))
    }

    async fn ensure_ready(&self, id: &SandboxId, image_override: Option<&str>) -> Result<()> {
        if self.session_vm(id).await.is_some() {
            return Ok(());
        }
        let permit = self.creation_permit(id).await;
        let _permit = permit.acquire_owned().await.map_err(|e| {
            Error::message(format!("firecracker: sandbox creation permit closed: {e}"))
        })?;
        if self.session_vm(id).await.is_some() {
            return Ok(());
        }

        if !firecracker_bin_available(Some(&self.fc.firecracker_bin)) {
            return Err(Error::message(format!(
                "firecracker: binary not found at {}",
                self.fc.firecracker_bin.display()
            )));
        }
        // curl is required for Firecracker API calls over Unix socket.
        if !super::containers::is_cli_available("curl") {
            return Err(Error::message(
                "firecracker: curl is required for API calls over Unix socket (install curl)",
            ));
        }
        if !self.fc.kernel_path.exists() {
            return Err(Error::message(format!(
                "firecracker: kernel not found at {}",
                self.fc.kernel_path.display()
            )));
        }
        if !self.fc.rootfs_path.exists() {
            return Err(Error::message(format!(
                "firecracker: rootfs not found at {}",
                self.fc.rootfs_path.display()
            )));
        }

        let (host_ip, guest_ip, subnet_idx) = self.allocate_subnet()?;
        let tap_name = format!("moltis-fc{subnet_idx}");

        let data_dir = moltis_config::data_dir();
        let vm_dir = data_dir.join("sandbox").join("firecracker").join(&id.key);
        std::fs::create_dir_all(&vm_dir)
            .map_err(|e| Error::message(format!("firecracker: failed to create VM dir: {e}")))?;
        let rootfs_copy = vm_dir.join("rootfs.ext4");
        let api_socket = vm_dir.join("api.sock");
        let _ = std::fs::remove_file(&api_socket);

        // Use pre-built rootfs if available (from build_image()), otherwise base.
        let source_rootfs = image_override
            .map(Path::new)
            .filter(|p| p.exists())
            .unwrap_or(&self.fc.rootfs_path);

        info!(%id, tap = tap_name, guest_ip, source = %source_rootfs.display(), "firecracker: booting VM");

        Self::copy_rootfs(source_rootfs, &rootfs_copy).await?;
        if let Err(e) = Self::create_tap(&tap_name, &host_ip).await {
            let _ = std::fs::remove_dir_all(&vm_dir);
            return Err(e);
        }

        let process = match self
            .boot_vm(&api_socket, &rootfs_copy, &tap_name, &guest_ip, &host_ip)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                Self::remove_tap(&tap_name).await;
                let _ = std::fs::remove_dir_all(&vm_dir);
                return Err(e);
            },
        };

        if let Err(e) = Self::wait_for_ssh(&guest_ip, &self.fc.ssh_key_path).await {
            Self::remove_tap(&tap_name).await;
            let mut process = process;
            let _ = process.kill().await;
            let _ = process.wait().await;
            let _ = std::fs::remove_dir_all(&vm_dir);
            return Err(e);
        }

        info!(%id, guest_ip, "firecracker: VM ready");

        self.active
            .write()
            .await
            .insert(id.key.clone(), FirecrackerVm {
                process,
                api_socket,
                tap_device: tap_name,
                guest_ip,
                rootfs_copy,
            });

        Ok(())
    }

    async fn exec(&self, id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        let (guest_ip, ssh_key) = self
            .session_vm(id)
            .await
            .ok_or_else(|| Error::message(format!("firecracker: no active VM for {id}")))?;

        Self::ssh_run(&guest_ip, &ssh_key, command, opts).await
    }

    async fn write_file(
        &self,
        id: &SandboxId,
        file_path: &str,
        content: &[u8],
    ) -> Result<Option<serde_json::Value>> {
        let (guest_ip, ssh_key) = self
            .session_vm(id)
            .await
            .ok_or_else(|| Error::message(format!("firecracker: no active VM for {id}")))?;

        let quoted_path = shell_words::quote(file_path);
        let remote_tmp = format!("{file_path}.moltis-{}", uuid::Uuid::new_v4());
        let quoted_tmp = shell_words::quote(&remote_tmp);
        let preflight = format!(
            "path={quoted_path}; parent=$(dirname \"$path\"); \
             if [ ! -d \"$parent\" ]; then exit {EXIT_PARENT_MISSING}; fi; \
             if [ -L \"$path\" ]; then exit {EXIT_SYMLINK}; fi"
        );
        let opts = ExecOpts {
            timeout: Duration::from_secs(30),
            ..Default::default()
        };
        let result = Self::ssh_run(&guest_ip, &ssh_key, &preflight, &opts).await?;
        match result.exit_code {
            0 => {},
            EXIT_PARENT_MISSING => {
                return Err(Error::message(format!(
                    "cannot resolve parent of '{file_path}': directory does not exist in sandbox"
                )));
            },
            EXIT_SYMLINK => return Ok(Some(Self::write_file_denied(file_path))),
            other => {
                let detail = if result.stderr.trim().is_empty() {
                    format!("firecracker write preflight exited with code {other}")
                } else {
                    result.stderr.trim().to_string()
                };
                return Err(Error::message(format!(
                    "firecracker write of '{file_path}' failed: {detail}"
                )));
            },
        }

        let mut local_temp = tempfile::NamedTempFile::new()
            .map_err(|e| Error::message(format!("firecracker: temp file create failed: {e}")))?;
        local_temp
            .write_all(content)
            .map_err(|e| Error::message(format!("firecracker: temp file write failed: {e}")))?;
        local_temp
            .flush()
            .map_err(|e| Error::message(format!("firecracker: temp file flush failed: {e}")))?;

        if let Err(e) = Self::scp_upload(&guest_ip, &ssh_key, local_temp.path(), &remote_tmp).await
        {
            let cleanup = format!("rm -f -- {quoted_tmp}");
            let _ = Self::ssh_run(&guest_ip, &ssh_key, &cleanup, &opts).await;
            return Err(e);
        }

        let finalize = format!(
            "path={quoted_path}; tmp={quoted_tmp}; \
             if [ -L \"$path\" ]; then rm -f \"$tmp\"; exit {EXIT_SYMLINK}; fi; \
             sync \"$tmp\" 2>/dev/null || sync; \
             if ! mv \"$tmp\" \"$path\"; then status=$?; rm -f \"$tmp\"; exit \"$status\"; fi"
        );
        let result = match Self::ssh_run(&guest_ip, &ssh_key, &finalize, &opts).await {
            Ok(result) => result,
            Err(e) => {
                let cleanup = format!("rm -f -- {quoted_tmp}");
                let _ = Self::ssh_run(&guest_ip, &ssh_key, &cleanup, &opts).await;
                return Err(e);
            },
        };
        match result.exit_code {
            0 => Ok(None),
            EXIT_SYMLINK => Ok(Some(Self::write_file_denied(file_path))),
            other => {
                let detail = if result.stderr.trim().is_empty() {
                    format!("firecracker write finalize exited with code {other}")
                } else {
                    result.stderr.trim().to_string()
                };
                Err(Error::message(format!(
                    "firecracker write of '{file_path}' failed: {detail}"
                )))
            },
        }
    }

    async fn cleanup(&self, id: &SandboxId) -> Result<()> {
        let permit = self.existing_creation_permit(id).await;
        let _permit = match permit {
            Some(permit) => Some(permit.acquire_owned().await.map_err(|e| {
                Error::message(format!("firecracker: sandbox creation permit closed: {e}"))
            })?),
            None => None,
        };

        // Take ownership and drop the lock immediately so concurrent
        // exec()/ensure_ready() calls for other sessions are not blocked
        // during the async teardown below.
        let vm = self.active.write().await.remove(&id.key);
        self.creation_permits.write().await.remove(&id.key);
        if let Some(mut vm) = vm {
            debug!(%id, guest_ip = vm.guest_ip, "firecracker: stopping VM");

            let _ = Self::fc_api_call(
                &vm.api_socket,
                "PUT",
                "/actions",
                &serde_json::json!({ "action_type": "SendCtrlAltDel" }),
            )
            .await;

            tokio::time::sleep(Duration::from_secs(2)).await;

            let _ = vm.process.kill().await;
            let _ = vm.process.wait().await;

            Self::remove_tap(&vm.tap_device).await;

            if let Some(parent) = vm.rootfs_copy.parent() {
                let _ = std::fs::remove_dir_all(parent);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_firecracker_sandbox_backend_name() {
        let sandbox = FirecrackerSandbox::new(
            SandboxConfig::default(),
            FirecrackerSandboxConfig::default(),
        );
        assert_eq!(sandbox.backend_name(), "firecracker");
        assert!(sandbox.is_real());
        assert!(sandbox.provides_fs_isolation());
        assert!(sandbox.is_isolated());
    }

    #[test]
    fn test_firecracker_config_defaults() {
        let config = FirecrackerSandboxConfig::default();
        assert_eq!(config.vcpus, 2);
        assert_eq!(config.memory_mb, 512);
        assert_eq!(config.firecracker_bin, PathBuf::from("firecracker"));
    }

    #[test]
    fn test_firecracker_bin_available_checks_configured_path() -> std::io::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let bin = tempdir.path().join("firecracker");
        std::fs::write(&bin, b"#!/bin/sh\n")?;

        assert!(firecracker_bin_available(Some(&bin)));
        assert!(!firecracker_bin_available(Some(
            &tempdir.path().join("missing-firecracker")
        )));
        Ok(())
    }

    #[test]
    fn test_resolve_firecracker_bin_prefers_configured_path() {
        let configured = PathBuf::from("/custom/firecracker");
        assert_eq!(resolve_firecracker_bin(Some(&configured)), configured);
    }

    #[test]
    fn test_scp_target_quotes_remote_path() {
        assert_eq!(
            FirecrackerSandbox::scp_target("172.16.1.2", "/home/sandbox/a b.txt"),
            "sandbox@172.16.1.2:'/home/sandbox/a b.txt'"
        );
    }

    #[test]
    fn test_subnet_allocation() -> Result<()> {
        let sandbox = FirecrackerSandbox::new(
            SandboxConfig::default(),
            FirecrackerSandboxConfig::default(),
        );
        let (host1, guest1, idx1) = sandbox.allocate_subnet()?;
        let (host2, guest2, idx2) = sandbox.allocate_subnet()?;

        assert_eq!(idx1, 1);
        assert_eq!(idx2, 2);
        assert_ne!(host1, host2);
        assert_ne!(guest1, guest2);
        assert!(host1.starts_with("172.16."));
        assert!(guest1.starts_with("172.16."));
        Ok(())
    }

    #[test]
    fn test_remote_shell_command_quotes_exec_env() -> Result<()> {
        let command = FirecrackerSandbox::remote_shell_command(
            "/home/sandbox/project dir",
            "printf '%s' \"$API_TOKEN\"",
            &[
                ("API_TOKEN".to_string(), "secret'value".to_string()),
                ("SESSION_ID".to_string(), "abc 123".to_string()),
            ],
        )?;

        assert_eq!(
            command,
            "env 'API_TOKEN=secret'\\''value' 'SESSION_ID=abc 123' sh -lc 'cd '\\''/home/sandbox/project dir'\\'' && printf '\\''%s'\\'' \"$API_TOKEN\"'"
        );
        Ok(())
    }

    #[test]
    fn test_remote_shell_command_without_env() -> Result<()> {
        let command = FirecrackerSandbox::remote_shell_command("/home/sandbox", "pwd", &[])?;

        assert_eq!(command, "env sh -lc 'cd /home/sandbox && pwd'");
        Ok(())
    }

    #[test]
    fn test_remote_shell_command_rejects_invalid_keys() {
        let result = FirecrackerSandbox::remote_shell_command("/home/sandbox", "env", &[(
            "BAD-KEY".to_string(),
            "value".to_string(),
        )]);

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_no_active_vm_returns_error() -> Result<()> {
        let sandbox = FirecrackerSandbox::new(
            SandboxConfig::default(),
            FirecrackerSandboxConfig::default(),
        );
        let id = SandboxId {
            scope: crate::sandbox::types::SandboxScope::Session,
            key: "test".into(),
        };
        let opts = ExecOpts::default();
        let result = sandbox.exec(&id, "echo hello", &opts).await;
        let Err(error) = result else {
            return Err(Error::message("expected exec without active VM to fail"));
        };
        assert!(error.to_string().contains("no active VM"));
        Ok(())
    }
}
