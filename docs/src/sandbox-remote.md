# Remote Sandbox Backends

When Docker is unavailable (cloud deploys, restricted environments), moltis can
use remote sandbox backends to provide isolated command execution via cloud APIs.

## Available Backends

| Backend | Provider | Isolation | Package Manager |
|---------|----------|-----------|-----------------|
| **Vercel Sandbox** | Vercel (managed) | Firecracker microVM | `dnf` (Amazon Linux 2023) |
| **Daytona** | Daytona (managed or self-hosted) | Cloud sandbox | `apt-get` (Ubuntu) |
| **Firecracker** | Self-hosted (Linux) | Local microVM | `apt-get` (Ubuntu) |

## Vercel Sandbox

Vercel Sandbox creates ephemeral Firecracker microVMs via the Vercel API.
Each session gets its own isolated VM with millisecond boot times.

### Configuration

Set environment variables:

```bash
VERCEL_TOKEN=ver_your_token_here
VERCEL_TEAM_ID=team_your_team_id    # optional but recommended
```

Or configure in `moltis.toml`:

```toml
[tools.exec.sandbox]
backend = "vercel"  # or leave "auto" for auto-detection

# Optional: customize Vercel sandbox settings
vercel_runtime = "node24"       # node24, node22, or python3.13
vercel_timeout_ms = 300000      # 5 minutes
vercel_vcpus = 2
```

### Getting Credentials

1. **Token**: Go to [vercel.com/account/tokens](https://vercel.com/account/tokens) → Create
2. **Project ID** (required): Create a project at [vercel.com/new](https://vercel.com/new), then get the ID from Project Settings → General → "Project ID"
3. **Team ID** (optional but recommended): Go to your team's Settings → General → scroll to "Team ID"

### How It Works

- `backend = "auto"` detects `VERCEL_TOKEN` when no local Docker is available
- Each session creates an ephemeral Firecracker microVM
- Commands execute via the Vercel REST API
- Files transfer via gzipped tar upload / raw read
- On cleanup, the sandbox is stopped (resources freed immediately)
- Snapshots cache pre-installed packages for fast subsequent boots

## Daytona

Daytona provides cloud sandboxes via a REST API. You can use the managed
service at `app.daytona.io` or self-host Daytona on your own infrastructure
(e.g., Proxmox, bare-metal Linux, Kubernetes).

### Configuration

Set environment variables:

```bash
DAYTONA_API_KEY=dyt_your_api_key_here
DAYTONA_API_URL=https://app.daytona.io/api  # default, change for self-hosted
```

Or configure in `moltis.toml`:

```toml
[tools.exec.sandbox]
backend = "daytona"  # or leave "auto" for auto-detection

# Daytona API settings
daytona_api_url = "https://app.daytona.io/api"  # change for self-hosted
daytona_target = "us"                            # optional target region
```

### Self-Hosted Daytona

If you run Daytona on your own infrastructure (Proxmox, bare-metal, etc.),
point the API URL to your instance:

```toml
[tools.exec.sandbox]
daytona_api_url = "https://daytona.your-server.local/api"
```

Or via environment variable:

```bash
DAYTONA_API_URL=https://daytona.your-server.local/api
```

This gives you full control over the sandbox infrastructure while still
using moltis's multi-backend routing and workspace sync.

### Getting Credentials

1. Sign up at [daytona.io](https://www.daytona.io) or deploy self-hosted
2. Generate an API key from the Daytona dashboard
3. Set `DAYTONA_API_KEY` in your environment

### How It Works

- `backend = "auto"` detects `DAYTONA_API_KEY` when no local Docker is available
- Each session creates an ephemeral cloud sandbox
- Commands execute via the toolbox REST API
- Files transfer via multipart upload / download
- On cleanup, the sandbox is deleted

## Local Firecracker

For Linux servers without Docker where you want VM-level isolation, the
Firecracker backend boots microVMs directly using the Firecracker hypervisor.

### Requirements

- Linux only (Firecracker requires KVM)
- `firecracker` binary installed
- Uncompressed Linux kernel (`vmlinux`)
- ext4 rootfs image with SSH server and `sandbox` user
- Root access or `CAP_NET_ADMIN` for TAP networking

### Configuration

```toml
[tools.exec.sandbox]
backend = "firecracker"

firecracker_bin = "/usr/local/bin/firecracker"
firecracker_kernel = "/opt/moltis/vmlinux"
firecracker_rootfs = "/opt/moltis/rootfs.ext4"
firecracker_ssh_key = "/opt/moltis/ssh_key"
firecracker_vcpus = 2
firecracker_memory_mb = 512
```

### How It Works

- Boots a Firecracker microVM in ~125ms
- Creates a dedicated TAP device per VM for networking
- Commands execute via SSH into the guest
- Pre-built rootfs caches packages (like Docker image building)
- On cleanup, the VM is shut down and TAP device removed

## Auto-Detection

When `backend = "auto"` (the default), moltis selects the sandbox backend
in this order:

1. **Local**: Apple Container → Podman → Docker → (next)
2. **Remote**: Vercel (if `VERCEL_TOKEN` set) → Daytona (if `DAYTONA_API_KEY` set)
3. **Fallback**: Restricted Host (rlimits only, no isolation)

## Multi-Backend Routing

Multiple backends can be active simultaneously. Per-session backend selection
allows different sessions to use different backends:

```json
{ "key": "session:heavy-compute", "sandboxBackend": "vercel" }
{ "key": "session:quick-test", "sandboxBackend": "docker" }
```

Configure backends in the **Settings → Sandboxes → Remote sandbox backends**
section of the web UI, or via environment variables and `moltis.toml`.

## Web UI Configuration

Navigate to **Settings → Sandboxes** and scroll to the "Remote sandbox backends"
section. Enter your API tokens and save — moltis will use them after restart.

## Package Provisioning

Remote sandboxes automatically install the same default packages configured for
local Docker sandboxes. The first session may take longer as packages are
installed, but subsequent sessions use cached images/snapshots:

| Backend | Caching Strategy |
|---------|-----------------|
| Vercel | Snapshot after first provisioning (instant subsequent boots) |
| Daytona | Runtime provisioning on first session |
| Firecracker | Pre-built rootfs with packages baked in |
