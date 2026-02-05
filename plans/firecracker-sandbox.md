# Firecracker Sandbox Backend

**Status:** Not implemented
**Priority:** Low (nice-to-have)
**Complexity:** High
**Platform:** Linux only (requires KVM)

## Overview

This document describes how to add Firecracker microVM support as a sandbox
backend for moltis on Linux. Firecracker provides hypervisor-level isolation
(the same technology powering AWS Lambda) with ~125ms boot times and <5 MiB
memory overhead per VM.

### Why Firecracker?

The current sandbox backends offer different isolation levels:

| Backend | Isolation | Platform |
|---------|-----------|----------|
| Docker | Namespace (shared kernel) | All |
| Apple Container | Hypervisor (separate kernel) | macOS 26+ |
| Cgroup | Resource limits only | Linux |
| **Firecracker** | **Hypervisor (separate kernel)** | **Linux** |

Firecracker would fill the gap: hypervisor-level isolation on Linux, matching
what Apple Container provides on macOS.

### Why It's Not Trivial

Unlike Docker (`docker exec`) or Apple Container (`container exec`),
Firecracker has **no built-in command execution capability**. To run commands
inside a Firecracker VM, you must:

1. Build a custom guest agent that runs inside the VM
2. Communicate with it via vsock (virtio socket)
3. Maintain kernel and rootfs images

This is significantly more work than the other backends.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│ Host                                                             │
│  ┌───────────────┐                      ┌────────────────────┐  │
│  │ moltis        │                      │ Firecracker VMM    │  │
│  │ ExecTool      │◄────Unix Socket─────►│                    │  │
│  │               │    /tmp/fc-XX.sock   │ API: localhost/... │  │
│  └───────┬───────┘                      └─────────┬──────────┘  │
│          │                                        │             │
│          │ vsock connect                          │ KVM         │
│          ▼                                        ▼             │
│  ┌───────────────┐                      ┌────────────────────┐  │
│  │ vsock proxy   │◄────────────────────►│ microVM guest      │  │
│  │ ./vm.sock     │      AF_VSOCK        │                    │  │
│  └───────────────┘                      │  ┌──────────────┐  │  │
│                                         │  │ guest-agent  │  │  │
│                                         │  │ :5000 vsock  │  │  │
│                                         │  └──────────────┘  │  │
│                                         └────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

### Components

1. **Firecracker binary** - the VMM process
2. **Guest kernel** - uncompressed vmlinux with virtio drivers
3. **Guest rootfs** - ext4 image with Linux userland + guest agent
4. **Guest agent** - Rust binary listening on vsock, runs commands
5. **Host client** - code in `FirecrackerSandbox` that talks to the agent

## Implementation Plan

### Phase 1: Guest Agent

Create a new crate `crates/firecracker-agent` that runs inside the VM.

```rust
// crates/firecracker-agent/src/main.rs

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::os::unix::net::VsockListener;
use std::process::{Command, Stdio};

const VSOCK_PORT: u32 = 5000;

#[derive(Deserialize)]
struct ExecRequest {
    command: String,
    working_dir: Option<String>,
    env: Vec<(String, String)>,
    timeout_secs: u64,
}

#[derive(Serialize)]
struct ExecResponse {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

fn main() -> std::io::Result<()> {
    // VMADDR_CID_ANY = -1 (0xFFFFFFFF)
    let listener = VsockListener::bind_with_cid_port(
        libc::VMADDR_CID_ANY as u32,
        VSOCK_PORT,
    )?;

    println!("guest-agent listening on vsock port {VSOCK_PORT}");

    for stream in listener.incoming() {
        let mut stream = stream?;

        // Read length-prefixed JSON request
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf) as usize;

        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload)?;

        let req: ExecRequest = serde_json::from_slice(&payload)?;

        // Run the command
        let mut cmd = Command::new("sh");
        cmd.args(["-c", &req.command]);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        if let Some(ref dir) = req.working_dir {
            cmd.current_dir(dir);
        }
        for (k, v) in &req.env {
            cmd.env(k, v);
        }

        let output = cmd.output()?;

        let response = ExecResponse {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code().unwrap_or(-1),
        };

        // Write length-prefixed JSON response
        let resp_bytes = serde_json::to_vec(&response)?;
        stream.write_all(&(resp_bytes.len() as u32).to_le_bytes())?;
        stream.write_all(&resp_bytes)?;
    }

    Ok(())
}
```

The agent needs to be compiled for the guest architecture (likely
`x86_64-unknown-linux-musl` for a static binary).

### Phase 2: Rootfs Image

Create a minimal rootfs with the agent:

```bash
#!/bin/bash
# scripts/build-firecracker-rootfs.sh

set -euo pipefail

ROOTFS_SIZE="512M"
ROOTFS_FILE="moltis-rootfs.ext4"
AGENT_BINARY="target/x86_64-unknown-linux-musl/release/firecracker-agent"

# Create empty ext4 image
dd if=/dev/zero of="$ROOTFS_FILE" bs=1M count=512
mkfs.ext4 "$ROOTFS_FILE"

# Mount and populate
MOUNT_DIR=$(mktemp -d)
sudo mount "$ROOTFS_FILE" "$MOUNT_DIR"

# Install Alpine Linux minimal
sudo apk --arch x86_64 -X http://dl-cdn.alpinelinux.org/alpine/latest-stable/main \
    -U --allow-untrusted --root "$MOUNT_DIR" --initdb \
    add alpine-base openrc busybox-initscripts

# Add common development tools
sudo apk --arch x86_64 -X http://dl-cdn.alpinelinux.org/alpine/latest-stable/main \
    --root "$MOUNT_DIR" add \
    git curl wget openssh-client jq

# Copy guest agent
sudo cp "$AGENT_BINARY" "$MOUNT_DIR/usr/local/bin/moltis-agent"
sudo chmod +x "$MOUNT_DIR/usr/local/bin/moltis-agent"

# Create init script to start agent on boot
sudo tee "$MOUNT_DIR/etc/init.d/moltis-agent" > /dev/null <<'EOF'
#!/sbin/openrc-run
command="/usr/local/bin/moltis-agent"
command_background=true
pidfile="/run/moltis-agent.pid"
EOF
sudo chmod +x "$MOUNT_DIR/etc/init.d/moltis-agent"
sudo chroot "$MOUNT_DIR" rc-update add moltis-agent default

# Set up networking (optional, for internet access)
sudo tee "$MOUNT_DIR/etc/network/interfaces" > /dev/null <<'EOF'
auto lo
iface lo inet loopback

auto eth0
iface eth0 inet dhcp
EOF

# Unmount
sudo umount "$MOUNT_DIR"
rmdir "$MOUNT_DIR"

echo "Rootfs created: $ROOTFS_FILE"
```

### Phase 3: Kernel

Download a pre-built kernel or build one with the required options:

```bash
# Download from Firecracker's CI (they provide tested kernels)
KERNEL_VERSION="5.10"
curl -fsSL -o vmlinux.bin \
    "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v${KERNEL_VERSION}/x86_64/vmlinux-${KERNEL_VERSION}.bin"
```

Required kernel config options:

```
CONFIG_VIRTIO_VSOCKETS=y
CONFIG_VIRTIO_BLK=y
CONFIG_VIRTIO_NET=y
CONFIG_EXT4_FS=y
```

### Phase 4: Host-Side Sandbox Implementation

```rust
// crates/tools/src/firecracker_sandbox.rs

use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::exec::{ExecOpts, ExecResult};
use crate::sandbox::{BuildImageResult, Sandbox, SandboxConfig, SandboxId};

const VSOCK_PORT: u32 = 5000;
const GUEST_CID: u32 = 3;

/// Firecracker microVM sandbox (Linux only, requires KVM).
pub struct FirecrackerSandbox {
    config: SandboxConfig,
    firecracker_bin: PathBuf,
    kernel_path: PathBuf,
    rootfs_path: PathBuf,
    vms: tokio::sync::RwLock<std::collections::HashMap<String, RunningVm>>,
}

struct RunningVm {
    process: Child,
    api_socket: PathBuf,
    vsock_path: PathBuf,
}

#[derive(Serialize)]
struct ExecRequest {
    command: String,
    working_dir: Option<String>,
    env: Vec<(String, String)>,
    timeout_secs: u64,
}

#[derive(Deserialize)]
struct ExecResponse {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

impl FirecrackerSandbox {
    pub fn new(
        config: SandboxConfig,
        firecracker_bin: PathBuf,
        kernel_path: PathBuf,
        rootfs_path: PathBuf,
    ) -> Self {
        Self {
            config,
            firecracker_bin,
            kernel_path,
            rootfs_path,
            vms: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    fn socket_dir(&self) -> PathBuf {
        std::env::temp_dir().join("moltis-firecracker")
    }

    async fn configure_vm(&self, api_socket: &Path, vsock_path: &Path) -> Result<()> {
        // Use HTTP client over Unix socket to configure Firecracker
        // See Firecracker API docs for full endpoint list

        // PUT /boot-source - kernel config
        // PUT /drives/rootfs - root filesystem
        // PUT /vsock - vsock device
        // PUT /machine-config - vcpu/memory
        // PUT /actions {"action_type": "InstanceStart"}

        todo!("implement HTTP API calls")
    }

    async fn wait_for_agent(&self, vsock_path: &Path) -> Result<()> {
        for _ in 0..50 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if self.ping_agent(vsock_path).await.is_ok() {
                return Ok(());
            }
        }
        anyhow::bail!("guest agent did not become ready within 5s")
    }

    async fn ping_agent(&self, vsock_path: &Path) -> Result<()> {
        let mut stream = UnixStream::connect(vsock_path).await?;
        stream
            .write_all(format!("CONNECT {VSOCK_PORT}\n").as_bytes())
            .await?;

        let mut buf = [0u8; 64];
        let n = stream.read(&mut buf).await?;
        let response = std::str::from_utf8(&buf[..n])?;

        if response.starts_with("OK ") {
            Ok(())
        } else {
            anyhow::bail!("unexpected vsock response: {response}")
        }
    }

    async fn exec_via_vsock(
        &self,
        vsock_path: &Path,
        command: &str,
        opts: &ExecOpts,
    ) -> Result<ExecResult> {
        let mut stream = UnixStream::connect(vsock_path).await?;

        // Connect to agent port
        stream
            .write_all(format!("CONNECT {VSOCK_PORT}\n").as_bytes())
            .await?;

        let mut buf = [0u8; 64];
        let n = stream.read(&mut buf).await?;
        let response = std::str::from_utf8(&buf[..n])?;
        if !response.starts_with("OK ") {
            anyhow::bail!("vsock connect failed: {response}");
        }

        // Send request
        let request = ExecRequest {
            command: command.to_string(),
            working_dir: opts.working_dir.as_ref().map(|p| p.display().to_string()),
            env: opts.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            timeout_secs: opts.timeout.as_secs(),
        };

        let payload = serde_json::to_vec(&request)?;
        stream.write_all(&(payload.len() as u32).to_le_bytes()).await?;
        stream.write_all(&payload).await?;

        // Read response
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await?;
        let len = u32::from_le_bytes(len_buf) as usize;

        let mut resp_buf = vec![0u8; len];
        stream.read_exact(&mut resp_buf).await?;

        let response: ExecResponse = serde_json::from_slice(&resp_buf)?;

        Ok(ExecResult {
            stdout: response.stdout,
            stderr: response.stderr,
            exit_code: response.exit_code,
        })
    }
}

#[async_trait]
impl Sandbox for FirecrackerSandbox {
    fn backend_name(&self) -> &'static str {
        "firecracker"
    }

    async fn ensure_ready(&self, id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
        let key = id.key.clone();

        if self.vms.read().await.contains_key(&key) {
            return Ok(());
        }

        let socket_dir = self.socket_dir();
        std::fs::create_dir_all(&socket_dir)?;

        let api_socket = socket_dir.join(format!("{key}.sock"));
        let vsock_path = socket_dir.join(format!("{key}.vsock"));

        let _ = std::fs::remove_file(&api_socket);
        let _ = std::fs::remove_file(&vsock_path);

        let process = Command::new(&self.firecracker_bin)
            .args(["--api-sock", &api_socket.display().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to spawn firecracker")?;

        // Wait for API socket
        for _ in 0..50 {
            if api_socket.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        self.configure_vm(&api_socket, &vsock_path).await?;
        self.wait_for_agent(&vsock_path).await?;

        self.vms.write().await.insert(key, RunningVm {
            process,
            api_socket,
            vsock_path,
        });

        Ok(())
    }

    async fn exec(&self, id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        let vms = self.vms.read().await;
        let vm = vms
            .get(&id.key)
            .ok_or_else(|| anyhow::anyhow!("VM not found: {}", id.key))?;

        self.exec_via_vsock(&vm.vsock_path, command, opts).await
    }

    async fn cleanup(&self, id: &SandboxId) -> Result<()> {
        if let Some(mut vm) = self.vms.write().await.remove(&id.key) {
            let _ = vm.process.kill().await;
            let _ = std::fs::remove_file(&vm.api_socket);
            let _ = std::fs::remove_file(&vm.vsock_path);
        }
        Ok(())
    }

    async fn build_image(
        &self,
        _base: &str,
        _packages: &[String],
    ) -> Result<Option<BuildImageResult>> {
        // TODO: Could build custom rootfs images with packages pre-installed
        Ok(None)
    }
}
```

### Phase 5: Integration

Update `select_backend` in `sandbox.rs`:

```rust
fn select_backend(config: SandboxConfig) -> Arc<dyn Sandbox> {
    match config.backend.as_str() {
        "docker" => Arc::new(DockerSandbox::new(config)),
        #[cfg(target_os = "macos")]
        "apple-container" => Arc::new(AppleContainerSandbox::new(config)),
        #[cfg(target_os = "linux")]
        "firecracker" => {
            // TODO: resolve paths from config or defaults
            let fc_bin = PathBuf::from("/usr/local/bin/firecracker");
            let kernel = PathBuf::from("/var/lib/moltis/vmlinux");
            let rootfs = PathBuf::from("/var/lib/moltis/rootfs.ext4");
            Arc::new(FirecrackerSandbox::new(config, fc_bin, kernel, rootfs))
        }
        _ => auto_detect_backend(config),
    }
}
```

## Configuration

Add to `moltis.toml`:

```toml
[tools.exec.sandbox]
backend = "firecracker"  # or "auto" to prefer it when available

[tools.exec.sandbox.firecracker]
binary = "/usr/local/bin/firecracker"
kernel = "/var/lib/moltis/vmlinux"
rootfs = "/var/lib/moltis/rootfs.ext4"
```

## Requirements

### Host Requirements

- Linux kernel with KVM support (`lsmod | grep kvm`)
- Access to `/dev/kvm` (user in `kvm` group or ACL set)
- Firecracker binary installed
- Kernel and rootfs images

### Kernel Requirements

```
CONFIG_VIRTIO_VSOCKETS=y
CONFIG_VIRTIO_BLK=y
CONFIG_VIRTIO_NET=y (optional, for networking)
CONFIG_EXT4_FS=y
```

### Rootfs Requirements

- Linux userland (Alpine recommended for size)
- moltis guest agent running on boot
- `/dev/vsock` device available

## Workspace Mounting

Unlike Docker, Firecracker doesn't support bind mounts. Options:

### Option A: Copy Files (Simplest)

Before starting the VM, copy workspace files into the rootfs. After execution,
sync changes back. This is what Docker Sandboxes does.

**Pros:** Simple, no kernel changes needed
**Cons:** Overhead for large workspaces, sync complexity

### Option B: virtio-block (Read-Only)

Mount the workspace directory as a second block device (read-only):

```rust
// In configure_vm():
// PUT /drives/workspace
// { "drive_id": "workspace", "path_on_host": workspace_path,
//   "is_root_device": false, "is_read_only": true }
```

Guest mounts it at a known path. Good for read-only access.

### Option C: virtiofs (Advanced)

Use virtiofs for shared filesystem access. Requires:

- `virtiofsd` daemon on host
- Kernel with virtiofs support
- More complex setup

Not recommended for initial implementation.

## Testing

```bash
# Check KVM access
ls -la /dev/kvm

# Test firecracker manually
./firecracker --api-sock /tmp/test.sock

# Run moltis with firecracker backend
MOLTIS_SANDBOX_BACKEND=firecracker moltis serve
```

## Security Considerations

### Jailer

For production, Firecracker should run inside its `jailer` wrapper which:

- Drops privileges
- Applies seccomp filters
- Creates cgroup/namespace isolation around the VMM itself

```bash
jailer --id myvm \
    --exec-file /usr/bin/firecracker \
    --uid 1000 --gid 1000 \
    -- --api-sock /run/firecracker.sock
```

### Network Isolation

By default, VMs have no network access. To enable:

1. Create TAP interface on host
2. Configure iptables for NAT
3. Add network device to VM config

For sandboxing untrusted code, network isolation (the default) is preferred.

## Dependencies

```toml
# Cargo.toml additions

[target.'cfg(target_os = "linux")'.dependencies]
tokio = { version = "1", features = ["net", "io-util"] }

# Optional: use fctools for higher-level API
# fctools = { version = "0.7", features = ["tokio-runtime", "nix-syscall-backend"] }
```

## References

- [Firecracker GitHub](https://github.com/firecracker-microvm/firecracker)
- [Getting Started Guide](https://github.com/firecracker-microvm/firecracker/blob/main/docs/getting-started.md)
- [vsock Documentation](https://github.com/firecracker-microvm/firecracker/blob/main/docs/vsock.md)
- [fctools Rust SDK](https://crates.io/crates/fctools)
- [firecracker-containerd](https://github.com/firecracker-microvm/firecracker-containerd)
- [E2B Infrastructure](https://github.com/e2b-dev/infra)

## Estimated Effort

| Phase | Description | Effort |
|-------|-------------|--------|
| 1 | Guest agent | 2-3 days |
| 2 | Rootfs image build | 1-2 days |
| 3 | Host sandbox impl | 3-4 days |
| 4 | Integration + testing | 2-3 days |
| 5 | Workspace mounting | 2-3 days |
| 6 | Jailer + production | 2-3 days |

**Total: ~2-3 weeks** for a production-ready implementation.

## Alternatives Considered

### gVisor

User-space kernel that intercepts syscalls. Can run as an OCI runtime
(`runsc`) with Docker. Simpler integration but weaker isolation than
hypervisor-based solutions.

### Kata Containers

Combines containers with lightweight VMs. Uses QEMU or Cloud Hypervisor.
Heavier than Firecracker but more feature-complete.

### QEMU microvm

QEMU's microvm machine type. More features than Firecracker but larger attack
surface and slower boot.

Firecracker was chosen for this plan because:

- Fastest boot time (~125ms)
- Smallest memory footprint (<5 MiB)
- Production-proven (AWS Lambda)
- Written in Rust (good ecosystem fit)
