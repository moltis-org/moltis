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
```

With `"auto"` (the default), Moltis picks the strongest available backend:

| Priority | Backend           | Platform | Isolation          |
|----------|-------------------|----------|--------------------|
| 1        | Apple Container   | macOS    | VM (Virtualization.framework) |
| 2        | Docker            | any      | Linux namespaces / cgroups    |
| 3        | none (host)       | any      | no isolation                  |

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

## No sandbox

If neither runtime is found, commands execute directly on the host. The
startup banner will show a warning. This is **not recommended** for untrusted
workloads.

## Per-session overrides

The web UI allows toggling sandboxing per session and selecting a custom
container image. These overrides persist across gateway restarts.

## Resource limits

```toml
[tools.exec.sandbox.resource_limits]
memory_limit = "512M"
cpu_quota = 1.0
pids_max = 256
```
