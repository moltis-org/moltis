# Sandbox Backends

Moltis runs LLM-generated commands inside containers to protect your host
system. The sandbox backend controls which container technology is used.

## Backend Selection

Configure in `moltis.toml`:

```toml
[tools.exec.sandbox]
backend = "auto"          # default — picks the best available
# backend = "docker"      # force Docker
# backend = "apple-container"  # force Apple Container (macOS only)
# backend = "wasm"        # force WASM sandbox (Wasmtime + WASI)
# backend = "restricted-host"  # env clearing + rlimits only
```

With `"auto"` (the default), Moltis picks the strongest available backend:

| Priority | Backend           | Platform | Isolation          |
|----------|-------------------|----------|--------------------|
| 1        | Apple Container   | macOS    | VM (Virtualization.framework) |
| 2        | Docker            | any      | Linux namespaces / cgroups    |
| 3        | Restricted Host   | any      | env clearing, rlimits (no filesystem isolation) |
| 4        | none (host)       | any      | no isolation                  |

The WASM backend (`backend = "wasm"`) is not in the auto-detect chain because
it cannot execute arbitrary shell commands — use it explicitly when you want
WASI-isolated execution.

## Apple Container (recommended on macOS)

[Apple Container](https://github.com/apple/container) runs each sandbox in a
lightweight virtual machine using Apple's Virtualization.framework. Every
container gets its own kernel, so a kernel exploit inside the sandbox cannot
reach the host — unlike Docker, which shares the host kernel.

### Install

Download the signed installer from GitHub:

```bash
# Download the installer package
gh release download --repo apple/container --pattern "container-installer-signed.pkg" --dir /tmp

# Install (requires admin)
sudo installer -pkg /tmp/container-installer-signed.pkg -target /

# First-time setup — downloads a default Linux kernel
container system start
```

Alternatively, build from source with `brew install container` (requires
Xcode 26+).

### Verify

```bash
container --version
# Run a quick test
container run --rm ubuntu echo "hello from VM"
```

Once installed, restart `moltis gateway` — the startup banner will show
`sandbox: apple-container backend`.

## Docker

Docker is supported on macOS, Linux, and Windows. On macOS it runs inside a
Linux VM managed by Docker Desktop, so it is reasonably isolated but adds more
overhead than Apple Container.

Install from https://docs.docker.com/get-docker/

### Docker Hardening

Docker containers launched by Moltis include the following security hardening
flags by default:

| Flag | Effect |
|------|--------|
| `--cap-drop ALL` | Drops all Linux capabilities |
| `--security-opt no-new-privileges` | Prevents privilege escalation via setuid/setgid binaries |
| `--tmpfs /tmp:rw,nosuid,size=256m` | Writable tmpfs for temp files (noexec on real root) |
| `--tmpfs /run:rw,nosuid,size=64m` | Writable tmpfs for runtime files |
| `--read-only` | Read-only root filesystem (prebuilt images only) |

The `--read-only` flag is applied only to prebuilt sandbox images (where
packages are already baked in). Non-prebuilt images need a writable root
filesystem for `apt-get` provisioning on first start.

## WASM Sandbox (Wasmtime + WASI)

The WASM sandbox provides real sandboxed execution using
[Wasmtime](https://wasmtime.dev/) with WASI. Commands execute in an isolated
filesystem tree with fuel metering and epoch-based timeout enforcement.

### How It Works

The WASM sandbox has two execution tiers:

**Tier 1 — Built-in commands** (~20 common coreutils implemented in Rust):
`echo`, `cat`, `ls`, `mkdir`, `rm`, `cp`, `mv`, `pwd`, `env`, `head`, `tail`,
`wc`, `sort`, `touch`, `which`, `true`, `false`, `test`/`[`, `basename`,
`dirname`.

These operate on a sandboxed directory tree, translating guest paths (e.g.
`/home/sandbox/file.txt`) to host paths under `~/.moltis/sandbox/wasm/<id>/`.
Paths outside the sandbox root are rejected.

Basic shell features are supported: `&&`, `||`, `;` sequences, `$VAR`
expansion, quoting via `shell-words`, and `>` / `>>` output redirects.

**Tier 2 — Real WASM module execution**: When the command references a `.wasm`
file, it is loaded and run via Wasmtime + WASI preview1 with full isolation:
preopened directories, fuel metering, epoch interruption, and captured I/O.

**Unknown commands** return exit code 127: "command not found in WASM sandbox".

### Filesystem Isolation

```
~/.moltis/sandbox/wasm/<session-key>/
  home/        preopened as /home/sandbox (rw)
  tmp/         preopened as /tmp (rw)
```

Home persistence is respected:
- `shared`: uses `data_dir()/sandbox/home/shared/wasm/`
- `session`: uses `data_dir()/sandbox/wasm/<session-id>/`
- `off`: per-session, cleaned up on `cleanup()`

### Resource Limits

- **Fuel metering**: `store.set_fuel(fuel_limit)` — limits WASM instruction
  count (Tier 2 only)
- **Epoch interruption**: background thread ticks epochs, store traps on
  deadline (Tier 2 only)
- **Memory**: `wasm_config.memory_reservation(bytes)` — Wasmtime memory limits
  (Tier 2 only)

### Configuration

```toml
[tools.exec.sandbox]
backend = "wasm"

# WASM-specific settings
wasm_fuel_limit = 1000000000       # instruction fuel (default: 1 billion)
wasm_epoch_interval_ms = 100       # epoch interruption interval (default: 100ms)

[tools.exec.sandbox.resource_limits]
memory_limit = "512M"    # Wasmtime memory reservation
```

### Limitations

- Built-in commands cover common coreutils but not a full shell
- No pipe support yet (planned via busybox.wasm in future)
- No network access from WASM modules
- `.wasm` modules must target WASI preview1

### When to Use

The WASM sandbox is a good fit when:

- You want filesystem-isolated execution without container overhead
- You need a sandboxed environment on platforms without Docker or Apple
  Container
- You are running `.wasm` modules and want fuel-metered, time-bounded execution

### Compile-Time Feature

The WASM sandbox is gated behind the `wasm` cargo feature, which is enabled by
default. To build without Wasmtime (saves ~30 MB binary size):

```bash
cargo build --release --no-default-features --features lightweight
```

When the feature is disabled and the config requests `backend = "wasm"`, Moltis
falls back to `restricted-host` with a warning.

## Restricted Host Sandbox

The restricted-host sandbox provides lightweight isolation by running commands
on the host via `sh -c` with environment clearing and `ulimit` resource
wrappers. This is the fallback when no container runtime is available.

### How It Works

When the restricted-host sandbox runs a command, it:

1. **Clears the environment** — all inherited environment variables are removed
2. **Sets a restricted PATH** — only `/usr/local/bin:/usr/bin:/bin`
3. **Sets HOME to `/tmp`** — prevents access to the user's home directory
4. **Applies resource limits** via shell `ulimit`:
   - `ulimit -u` (max processes) from `pids_max` config (default: 256)
   - `ulimit -n 1024` (max open files)
   - `ulimit -t` (CPU seconds) from `cpu_quota` config (default: 300s)
   - `ulimit -v` (virtual memory) from `memory_limit` config (default: 512M)
5. **Enforces a timeout** via `tokio::time::timeout`

User-specified environment variables from `opts.env` are re-applied after the
environment is cleared, so the LLM tool can still pass required variables.

### Limitations

- No filesystem isolation — commands run on the host filesystem
- No network isolation — commands can make network requests
- `ulimit` enforcement is best-effort
- No image building — `moltis sandbox build` returns immediately

For production use with untrusted workloads, prefer Apple Container or Docker.

## No sandbox

If no runtime is found (and the `wasm` feature is disabled), commands execute
directly on the host. The startup banner will show a warning. This is **not
recommended** for untrusted workloads.

## Failover Chain

Moltis wraps the primary sandbox backend with automatic failover:

- **Apple Container → Docker → Restricted Host**: if Apple Container enters a
  corrupted state (stale metadata, missing config, VM boot failure), Moltis
  fails over to Docker. If Docker is unavailable, it uses restricted-host.
- **Docker → Restricted Host**: if Docker loses its daemon connection during a
  session, Moltis fails over to the restricted-host sandbox.

Failover is sticky for the lifetime of the gateway process — once triggered,
all subsequent commands use the fallback backend. Restart the gateway to retry
the primary backend.

Failover triggers:

| Primary | Triggers |
|---------|----------|
| Apple Container | `config.json missing`, `VM never booted`, `NSPOSIXErrorDomain Code=22`, service errors |
| Docker | `cannot connect to the docker daemon`, `connection refused`, `is the docker daemon running` |

## Per-session overrides

The web UI allows toggling sandboxing per session and selecting a custom
container image. These overrides persist across gateway restarts.

## Home persistence

By default, `/home/sandbox` is persisted in a shared host folder so that CLI
auth/config files survive container recreation. You can change this with
`home_persistence`:

```toml
[tools.exec.sandbox]
home_persistence = "session"   # "off", "session", or "shared" (default)
# shared_home_dir = "/path/to/shared-home"  # optional, used when mode is "shared"
```

- `off`: no home mount, container home is ephemeral
- `session`: mount a per-session host folder to `/home/sandbox`
- `shared`: mount one shared host folder to `/home/sandbox` for all sessions
  (defaults to `data_dir()/sandbox/home/shared`, or `shared_home_dir` if set)

Moltis stores persisted homes under `data_dir()/sandbox/home/`.

> **Note**: Home persistence applies to Docker, Apple Container, and WASM
> backends. The restricted-host backend uses `HOME=/tmp` and does not mount
> persistent storage.

## Resource limits

```toml
[tools.exec.sandbox.resource_limits]
memory_limit = "512M"
cpu_quota = 1.0
pids_max = 256
```

How resource limits are applied depends on the backend:

| Limit | Docker | Apple Container | WASM | Restricted Host | cgroup (Linux) |
|-------|--------|-----------------|------|-----------------|----------------|
| `memory_limit` | `--memory` | `--memory` | Wasmtime reservation | `ulimit -v` | `MemoryMax=` |
| `cpu_quota` | `--cpus` | `--cpus` | epoch timeout | `ulimit -t` (seconds) | `CPUQuota=` |
| `pids_max` | `--pids-limit` | `--pids-limit` | n/a | `ulimit -u` | `TasksMax=` |

## Comparison

| Feature | Apple Container | Docker | WASM | Restricted Host | none |
|---------|----------------|--------|------|-----------------|------|
| Filesystem isolation | ✅ VM boundary | ✅ namespaces | ✅ sandboxed tree | ❌ host FS | ❌ |
| Network isolation | ✅ | ✅ | ✅ (no network) | ❌ | ❌ |
| Kernel isolation | ✅ separate kernel | ❌ shared kernel | ✅ WASM VM | ❌ | ❌ |
| Environment isolation | ✅ | ✅ | ✅ | ✅ cleared + restricted | ❌ |
| Resource limits | ✅ | ✅ | ✅ fuel + epoch | ✅ ulimit | ❌ |
| Image building | ✅ (via Docker) | ✅ | ❌ | ❌ | ❌ |
| Shell commands | ✅ full shell | ✅ full shell | ~20 built-ins | ✅ full shell | ✅ full shell |
| Platform | macOS 26+ | any | any | any | any |
| Overhead | low | medium | minimal | minimal | none |
