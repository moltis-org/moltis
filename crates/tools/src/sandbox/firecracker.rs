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
    path::{Path, PathBuf},
    time::Duration,
};

use {
    async_trait::async_trait,
    tokio::sync::RwLock,
    tracing::{debug, info, warn},
};

use crate::{
    error::{Error, Result},
    exec::{ExecOpts, ExecResult},
    sandbox::{
        file_system::SandboxReadResult,
        types::{Sandbox, SandboxConfig, SandboxId},
    },
};

const GUEST_USER: &str = "sandbox";
const SUBNET_BASE: &str = "172.16";
const FC_WORKSPACE: &str = "/home/sandbox";

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
            firecracker_bin: PathBuf::from("/usr/local/bin/firecracker"),
            kernel_path: PathBuf::from("/opt/moltis/vmlinux"),
            rootfs_path: PathBuf::from("/opt/moltis/rootfs.ext4"),
            ssh_key_path: PathBuf::from("/opt/moltis/ssh_key"),
            vcpus: 2,
            memory_mb: 512,
        }
    }
}

/// Firecracker sandbox backend.
pub struct FirecrackerSandbox {
    #[allow(dead_code)]
    config: SandboxConfig,
    fc: FirecrackerSandboxConfig,
    active: RwLock<HashMap<String, FirecrackerVm>>,
    subnet_counter: std::sync::atomic::AtomicU16,
}

impl FirecrackerSandbox {
    pub fn new(config: SandboxConfig, fc: FirecrackerSandboxConfig) -> Self {
        Self {
            config,
            fc,
            active: RwLock::new(HashMap::new()),
            subnet_counter: std::sync::atomic::AtomicU16::new(1),
        }
    }

    fn allocate_subnet(&self) -> (String, String, u16) {
        let idx = self
            .subnet_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let third = idx / 64;
        let fourth_base = (idx % 64) * 4;
        let host_ip = format!("{SUBNET_BASE}.{third}.{}", fourth_base + 1);
        let guest_ip = format!("{SUBNET_BASE}.{third}.{}", fourth_base + 2);
        (host_ip, guest_ip, idx)
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
        if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&stdout) {
            if resp.get("fault_message").is_some() {
                return Err(Error::message(format!(
                    "firecracker: API error on {method} {path}: {stdout}"
                )));
            }
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
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| Error::message(format!("firecracker: failed to spawn: {e}")))?;

        // Wait for API socket.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !api_socket.exists() {
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::message(
                    "firecracker: API socket did not appear within 5s",
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

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
        .await?;

        Ok(child)
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

            if let Ok(output) = result {
                if output.status.success() {
                    return Ok(());
                }
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(Error::message(
                    "firecracker: SSH did not become available within 30s",
                ));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
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

        let quoted_cwd = cwd.replace('\'', "'\\''");
        let full_cmd = format!("cd '{quoted_cwd}' && {command}");

        let output = tokio::process::Command::new("ssh")
            .args([
                "-i",
                &ssh_key.display().to_string(),
                "-o",
                "StrictHostKeyChecking=no",
                "-o",
                "UserKnownHostsFile=/dev/null",
                "-o",
                "BatchMode=yes",
                &format!("{GUEST_USER}@{guest_ip}"),
                "sh",
                "-c",
                &full_cmd,
            ])
            .output()
            .await
            .map_err(|e| Error::message(format!("firecracker: SSH run failed: {e}")))?;

        let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();
        stdout.truncate(stdout.floor_char_boundary(opts.max_output_bytes));
        stderr.truncate(stderr.floor_char_boundary(opts.max_output_bytes));

        Ok(ExecResult {
            stdout,
            stderr,
            exit_code: output.status.code().unwrap_or(-1),
        })
    }

    async fn session_vm(&self, id: &SandboxId) -> Option<(String, PathBuf)> {
        self.active
            .read()
            .await
            .get(&id.key)
            .map(|vm| (vm.guest_ip.clone(), self.fc.ssh_key_path.clone()))
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

        // Boot a temporary VM, install packages, shut down, keep the rootfs.
        let build_id = SandboxId {
            scope: super::types::SandboxScope::Session,
            key: format!("build-{tag}"),
        };
        let temp_rootfs = images_dir.join(format!("{tag}.building.ext4"));
        Self::copy_rootfs(&self.fc.rootfs_path, &temp_rootfs).await?;

        let (host_ip, guest_ip, subnet_idx) = self.allocate_subnet();
        let tap_name = format!("moltis-fc{subnet_idx}");
        let api_socket = images_dir.join(format!("{tag}.sock"));
        let _ = std::fs::remove_file(&api_socket);

        Self::create_tap(&tap_name, &host_ip).await?;

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
            Self::remove_tap(&tap_name).await;
            let _ = process.kill().await;
            let _ = std::fs::remove_file(&temp_rootfs);
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
        let result = Self::ssh_run(&guest_ip, &self.fc.ssh_key_path, &install_cmd, &opts).await?;
        if result.exit_code != 0 {
            warn!(
                tag,
                exit_code = result.exit_code,
                "firecracker: package install during image build failed (continuing)"
            );
        }

        // Graceful shutdown.
        let _ = Self::fc_api_call(
            &api_socket,
            "PUT",
            "/actions",
            &serde_json::json!({ "action_type": "SendCtrlAltDel" }),
        )
        .await;
        tokio::time::sleep(Duration::from_secs(3)).await;
        let _ = process.kill().await;
        let _ = process.wait().await;
        Self::remove_tap(&tap_name).await;
        let _ = std::fs::remove_file(&api_socket);

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

        if !self.fc.firecracker_bin.exists() {
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

        let (host_ip, guest_ip, subnet_idx) = self.allocate_subnet();
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
            .map(std::path::Path::new)
            .filter(|p| p.exists())
            .unwrap_or(&self.fc.rootfs_path);

        info!(%id, tap = tap_name, guest_ip, source = %source_rootfs.display(), "firecracker: booting VM");

        Self::copy_rootfs(source_rootfs, &rootfs_copy).await?;
        Self::create_tap(&tap_name, &host_ip).await?;

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
            drop(process);
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

    async fn cleanup(&self, id: &SandboxId) -> Result<()> {
        // Take ownership and drop the lock immediately so concurrent
        // exec()/ensure_ready() calls for other sessions are not blocked
        // during the async teardown below.
        let vm = self.active.write().await.remove(&id.key);
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
        assert_eq!(
            config.firecracker_bin,
            PathBuf::from("/usr/local/bin/firecracker")
        );
    }

    #[test]
    fn test_subnet_allocation() {
        let sandbox = FirecrackerSandbox::new(
            SandboxConfig::default(),
            FirecrackerSandboxConfig::default(),
        );
        let (host1, guest1, idx1) = sandbox.allocate_subnet();
        let (host2, guest2, idx2) = sandbox.allocate_subnet();

        assert_eq!(idx1, 1);
        assert_eq!(idx2, 2);
        assert_ne!(host1, host2);
        assert_ne!(guest1, guest2);
        assert!(host1.starts_with("172.16."));
        assert!(guest1.starts_with("172.16."));
    }

    #[tokio::test]
    async fn test_no_active_vm_returns_error() {
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
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no active VM"));
    }
}
