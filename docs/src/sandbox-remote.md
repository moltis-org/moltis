# Remote Sandbox Backends

When Docker is unavailable (cloud deploys, restricted environments), moltis can
use remote sandbox backends to provide isolated command execution via cloud APIs.

## Available Backends

| Backend | Provider | Isolation | Package Manager |
|---------|----------|-----------|-----------------|
| **Vercel Sandbox** | Vercel (managed) | Firecracker microVM | `dnf` (Amazon Linux 2023) |
| **Daytona** | Daytona (managed or self-hosted) | Cloud sandbox | `apt-get` (Ubuntu) |
| **Coder** | Coder (managed or self-hosted) | Coder workspace template | Template-defined |
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

## Coder

Coder creates ephemeral workspaces from your Coder templates. Moltis uses the
Coder REST API for lifecycle operations and the workspace-agent reconnecting PTY
WebSocket for command execution. Workspaces created by Moltis are deleted on
cleanup.

### Configuration

The simplest setup is **Settings → Sandboxes → Coder**. Enter the deployment
URL, session token, and either a template ID or template name. Organization,
user, workspace prefix, workspace TTL, and size/preset are available on the same
tab. Environment-managed `CODER_URL` and `CODER_SESSION_TOKEN` values are shown
as read-only only when those nonempty aliases supply the effective value. An
explicit `moltis.toml` value takes precedence and remains editable even when a
stale alias is present. Environment values are never copied into `moltis.toml`
when other fields are saved.

Set environment variables:

```bash
CODER_URL=https://coder.example.com
CODER_SESSION_TOKEN=coder_your_token_here
CODER_ORGANIZATION=default          # optional when template_id is configured
CODER_TEMPLATE_NAME=moltis-devbox   # or configure coder_template_id
```

Or configure in `moltis.toml`:

```toml
[tools.exec.sandbox]
backend = "coder"  # or leave "auto" for auto-detection when no local runtime exists

coder_url = "https://coder.example.com"
coder_token = "${CODER_SESSION_TOKEN}"
coder_organization = "default"
coder_user = "me"
coder_template_name = "moltis-devbox"
coder_workspace_prefix = "moltis"
coder_ttl_ms = 300000
coder_size = "medium"

[tools.exec.sandbox.coder_template_presets]
small = "small"
medium = "medium"
large = "large"
xlarge = "xlarge"
```

`coder_url` must be an absolute HTTPS URL without user information, a query
string, or a fragment. Plain HTTP is accepted only for `localhost` or a literal
loopback address such as `127.0.0.1` or `[::1]`; private network addresses and
ordinary hostnames still require HTTPS. This policy is enforced by static config
diagnostics and by the web API before it persists a UI save.

`coder_ttl_ms` must be zero or a positive whole number of milliseconds. A value
of zero disables Coder's automatic workspace shutdown; omitting the field leaves
the template or deployment default in effect. Negative values are rejected by
config diagnostics and by the web API before persistence.

### Template Presets

`coder_size` selects an entry from `coder_template_presets`. Each value may be
either a Coder template preset name or a preset UUID. Names are resolved against
the active template version before workspace creation.

Use `coder_parameter_values` for advanced template parameters that are not
represented by presets:

```toml
[tools.exec.sandbox.coder_parameter_values]
region = "us"
```

The web form covers the core Coder fields. The TOML blocks above are the
advanced escape hatch for `coder_template_presets`, `coder_parameter_values`,
or any newly supported Coder option that does not yet have a dedicated control.

### How It Works

- `backend = "auto"` considers Coder available only when the effective config has a nonempty URL, a non-whitespace token, and either a template ID or template name
- Each session creates an ephemeral Coder workspace
- Moltis polls workspace/build/agent lifecycle state every two seconds for up to
  ten minutes. Only an explicit agent lifecycle state of `ready` is accepted, so
  commands do not race the template's startup script. `start_timeout`, a missing
  lifecycle state, and every other non-`ready` state are never treated as usable.
  Failed or canceled builds and terminal agent states fail immediately; other
  non-ready responses eventually fail at the creation deadline
- Commands execute via Coder's reconnecting PTY WebSocket
- On cleanup, the Coder workspace is deleted

Concurrent setup calls for one session are serialized; callers waiting on the
same session reuse the first ready workspace instead of creating duplicates.
Readiness polling itself retries pending lifecycle states until the ten-minute
creation deadline. If startup fails, Moltis requests deletion immediately. When
that cleanup request also fails, the provisional workspace remains tracked so a
later command can revalidate or restart it instead of creating an untracked
duplicate. Cleanup similarly retains a workspace in the active map until Coder
confirms deletion (a `404` also counts as already deleted), allowing a later
cleanup call to retry rather than silently losing lifecycle ownership.

The Coder HTTP client has a five-minute request timeout. Workspace creation has
the separate ten-minute readiness deadline above. Each command uses the normal
Moltis execution timeout, and `coder_ttl_ms` is Coder's workspace autostop TTL;
it does not change API, readiness, or command timeouts.

### Compatibility

Moltis does not currently guarantee a minimum Coder release or maintain a
version compatibility matrix. The integration requires the Coder v2 workspace,
build, workspace-agent, and reconnecting PTY APIs used above. In particular, the
workspace-agent response must include lifecycle state `ready`; deployments that
omit that state or expose an incompatible lifecycle schema fail closed rather
than running commands in a workspace of unknown readiness.

### Command and File Transport

Coder exposes no REST endpoint for a workspace filesystem, so commands and file
payloads both travel over the agent PTY. The PTY URL's `command` parameter
carries only a fixed ~200 byte bootstrap; the real script is streamed to the
workspace on the PTY stdin channel. Payload size is therefore bounded by memory
rather than by URL length, which is what makes `Write` and workspace sync work:

| Limit | Value |
|-------|-------|
| Single `Write` through the PTY stream | 64 MB |
| Workspace sync transfer (in or out) | 16 MB |

Sync is capped lower than a single `Write` because sync-out reads the workspace
tarball back as base64 on stdout, and that expansion has to fit the sandbox
file service's output budget. Exceeding either limit produces an explicit
"too large" error rather than a truncated transfer.

The bootstrap puts the terminal into raw mode before any payload is streamed,
which disables echo and `ONLCR` so the marker framing around stdout, stderr,
and the exit code stays parseable. The PTY window is reported as 1000×200 so
that programs which self-format to `COLUMNS` do not wrap their own output.

### Workspace Names

Coder validates workspace names against `^[a-zA-Z0-9]+(-[a-zA-Z0-9]+)*$` with a
32 character limit. Moltis derives the name deterministically from
`coder_workspace_prefix` and the session key, lowercasing and collapsing every
other character to `-`, then always appending a stable hash of the original
prefix and session key. The readable portion is truncated as needed to fit. The
same pair therefore produces the same name across restarts, while values that
normalize to the same slug or share a long prefix remain distinct.

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
2. **Remote**: Vercel (if `VERCEL_TOKEN` set) → Daytona (if `DAYTONA_API_KEY` set) → Coder (if URL, token, and a template ID or name are configured)
3. **Fallback**: Restricted Host (rlimits only, no isolation)

## Multi-Backend Routing

Multiple backends can be active simultaneously. Per-session backend selection
allows different sessions to use different backends:

```json
{ "key": "session:heavy-compute", "sandboxBackend": "vercel" }
{ "key": "session:quick-test", "sandboxBackend": "docker" }
```

Configure backends in the backend-specific tabs under **Settings → Sandboxes**,
or via environment variables and `moltis.toml`.

## Web UI Configuration

Navigate to **Settings → Sandboxes**, choose **Vercel**, **Daytona**, or
**Coder**, enter the required credentials and provider fields, then save. The
Coder form preserves an existing session token when the token field is left
blank and uses the new settings after restart. Coder saves with an insecure or
malformed URL are rejected without changing the on-disk config.

## Package Provisioning

Remote sandboxes automatically install the same default packages configured for
local Docker sandboxes. The first session may take longer as packages are
installed, but subsequent sessions use cached images/snapshots:

| Backend | Caching Strategy |
|---------|-----------------|
| Vercel | Snapshot after first provisioning (instant subsequent boots) |
| Daytona | Runtime provisioning on first session |
| Coder | Template-defined image/packages (Moltis waits for agent lifecycle `ready`) |
| Firecracker | Pre-built rootfs with packages baked in |
