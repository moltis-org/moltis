# Running Moltis in Docker

Moltis is available as a multi-architecture Docker image supporting both
`linux/amd64` and `linux/arm64`. The image is published to GitHub Container
Registry on every release.

## Quick Start

```bash
docker run -d \
  --name moltis \
  -p 13131:13131 \
  -p 13132:13132 \
  -p 1455:1455 \
  -v moltis-config:/home/moltis/.config/moltis \
  -v moltis-data:/home/moltis/.moltis \
  -v /var/run/docker.sock:/var/run/docker.sock \
  ghcr.io/moltis-org/moltis:latest
```

Open https://localhost:13131 in your browser and configure your LLM provider to start chatting.

### Ports

| Port | Purpose |
|------|---------|
| 13131 | Gateway (HTTPS) — web UI, API, WebSocket |
| 13132 | HTTP — CA certificate download for TLS trust |
| 1455 | OAuth callback — required for OpenAI Codex and other providers with pre-registered redirect URIs |

### Trusting the TLS certificate

Moltis generates a self-signed CA on first run. Browsers will show a security
warning until you trust this CA. Port 13132 serves the certificate over plain
HTTP so you can download it:

```bash
# Download the CA certificate
curl -o moltis-ca.pem http://localhost:13132/certs/ca.pem

# macOS — add to system Keychain and trust it
sudo security add-trusted-cert -d -r trustRoot \
  -k /Library/Keychains/System.keychain moltis-ca.pem

# Linux (Debian/Ubuntu)
sudo cp moltis-ca.pem /usr/local/share/ca-certificates/moltis-ca.crt
sudo update-ca-certificates
```

After trusting the CA, restart your browser. The warning will not appear again
(the CA persists in the mounted config volume).

```admonish note
When accessing from localhost, no authentication is required. If you access Moltis from a different machine (e.g., over the network), a setup code is printed to the container logs for authentication setup:

~~~bash
docker logs moltis
~~~
```

## Volume Mounts

Moltis uses two directories that should be persisted:

| Path | Contents |
|------|----------|
| `/home/moltis/.config/moltis` | Configuration files: `moltis.toml`, `credentials.json`, `mcp-servers.json` |
| `/home/moltis/.moltis` | Runtime data: databases, sessions, memory files, logs |

You can use named volumes (as shown above) or bind mounts to local directories
for easier access to configuration files:

```bash
docker run -d \
  --name moltis \
  -p 13131:13131 \
  -p 13132:13132 \
  -p 1455:1455 \
  -v ./config:/home/moltis/.config/moltis \
  -v ./data:/home/moltis/.moltis \
  -v /var/run/docker.sock:/var/run/docker.sock \
  ghcr.io/moltis-org/moltis:latest
```

With bind mounts, you can edit `config/moltis.toml` directly on the host.

## Docker Socket (Sandbox Execution)

Moltis runs LLM-generated shell commands inside isolated containers for
security. When Moltis itself runs in a container, it needs access to the host's
container runtime to create these sandbox containers.

**Without the socket mount**, sandbox execution is disabled. The agent will
still work for chat-only interactions, but any tool that runs shell commands
will fail.

```bash
# Required for sandbox execution
-v /var/run/docker.sock:/var/run/docker.sock
```

### Security Consideration

Mounting the Docker socket gives the container full access to the Docker
daemon. This is equivalent to root access on the host for practical purposes.
Only run Moltis containers from trusted sources (official images from
`ghcr.io/moltis-org/moltis`).

If you cannot mount the Docker socket, Moltis will run in "no sandbox" mode —
commands execute directly inside the Moltis container itself, which provides
no isolation.

## Docker Compose

See [`examples/docker-compose.yml`](../examples/docker-compose.yml) for a
complete example:

```yaml
services:
  moltis:
    image: ghcr.io/moltis-org/moltis:latest
    container_name: moltis
    restart: unless-stopped
    ports:
      - "13131:13131"
      - "13132:13132"
      - "1455:1455"   # OAuth callback (OpenAI Codex, etc.)
    volumes:
      - ./config:/home/moltis/.config/moltis
      - ./data:/home/moltis/.moltis
      - /var/run/docker.sock:/var/run/docker.sock
```

### Coolify (Hetzner/VPS)

For Coolify service stacks, use
[`examples/docker-compose.coolify.yml`](../examples/docker-compose.coolify.yml).
It is preconfigured for reverse-proxy deployments (`--no-tls`) and includes
the Docker socket mount for sandboxed command execution.

Key points:

- Set `MOLTIS_PASSWORD` in the Coolify UI before first deploy.
- Set `SERVICE_FQDN_MOLTIS_13131` to your app domain.
- Keep Moltis in `--no-tls` mode behind Coolify's reverse proxy. If requests
  are redirected to `:13131`, check that TLS is disabled in Moltis.
- Keep `/var/run/docker.sock:/var/run/docker.sock` mounted if you want sandbox
  isolation for exec tools.

Start with:

```bash
docker compose up -d
docker compose logs -f moltis  # watch for startup messages
```

## Podman Support

Moltis works with Podman using its Docker-compatible API. Mount the Podman
socket instead of the Docker socket:

```bash
# Podman rootless
podman run -d \
  --name moltis \
  -p 13131:13131 \
  -p 13132:13132 \
  -p 1455:1455 \
  -v moltis-config:/home/moltis/.config/moltis \
  -v moltis-data:/home/moltis/.moltis \
  -v /run/user/$(id -u)/podman/podman.sock:/var/run/docker.sock \
  ghcr.io/moltis-org/moltis:latest

# Podman rootful
podman run -d \
  --name moltis \
  -p 13131:13131 \
  -p 13132:13132 \
  -p 1455:1455 \
  -v moltis-config:/home/moltis/.config/moltis \
  -v moltis-data:/home/moltis/.moltis \
  -v /run/podman/podman.sock:/var/run/docker.sock \
  ghcr.io/moltis-org/moltis:latest
```

You may need to enable the Podman socket service first:

```bash
# Rootless
systemctl --user enable --now podman.socket

# Rootful
sudo systemctl enable --now podman.socket
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `MOLTIS_CONFIG_DIR` | Override config directory (default: `~/.config/moltis`) |
| `MOLTIS_DATA_DIR` | Override data directory (default: `~/.moltis`) |

Example:

```bash
docker run -d \
  --name moltis \
  -p 13131:13131 \
  -p 13132:13132 \
  -p 1455:1455 \
  -e MOLTIS_CONFIG_DIR=/config \
  -e MOLTIS_DATA_DIR=/data \
  -v ./config:/config \
  -v ./data:/data \
  -v /var/run/docker.sock:/var/run/docker.sock \
  ghcr.io/moltis-org/moltis:latest
```

### API Keys and the `[env]` Section

Features like web search (Brave), embeddings, and LLM provider API calls read
keys from process environment variables (`std::env::var`). In Docker, there are
two ways to provide these:

**Option 1: `docker -e` flags** (takes precedence)

```bash
docker run -d \
  --name moltis \
  -e BRAVE_API_KEY=your-key \
  -e OPENROUTER_API_KEY=sk-or-... \
  ...
  ghcr.io/moltis-org/moltis:latest
```

**Option 2: `[env]` section in `moltis.toml`**

Add an `[env]` section to your config file. These variables are injected into
the Moltis process at startup, making them available to all features:

```toml
[env]
BRAVE_API_KEY = "your-brave-key"
OPENROUTER_API_KEY = "sk-or-..."
```

If a variable is set both via `docker -e` and `[env]`, the Docker/host
environment value wins — `[env]` never overwrites existing variables.

```admonish info title="Settings UI env vars"
Environment variables set through the Settings UI (Settings > Environment)
are stored in SQLite. At startup, Moltis injects them into the process
environment so they are available to all features (search, embeddings,
provider API calls), not just sandbox commands.

Precedence order (highest wins):
1. Host / `docker -e` environment variables
2. Config file `[env]` section
3. Settings UI environment variables
```

## Building Locally

To build the Docker image from source:

```bash
# Single architecture (current platform)
docker build -t moltis:local .

# Multi-architecture (requires buildx)
docker buildx build --platform linux/amd64,linux/arm64 -t moltis:local .
```

## OrbStack

OrbStack on macOS works identically to Docker — use the same socket path
(`/var/run/docker.sock`). OrbStack's lightweight Linux VM provides good
isolation with lower resource usage than Docker Desktop.

## Troubleshooting

### "Cannot connect to Docker daemon"

The Docker socket is not mounted or the Moltis user doesn't have permission
to access it. Verify:

```bash
docker exec moltis ls -la /var/run/docker.sock
```

### Setup code not appearing in logs (for network access)

The setup code only appears when accessing from a non-localhost address. If you're accessing from the same machine via `localhost`, no setup code is needed. For network access, wait a few seconds for the gateway to start, then check logs:

```bash
docker logs moltis 2>&1 | grep -i setup
```

### OAuth authentication error (OpenAI Codex)

If clicking **Connect** for OpenAI Codex shows "unknown_error" on OpenAI's
page, port 1455 is not reachable from your browser. Make sure you published it:

```bash
-p 1455:1455
```

If you're running Moltis on a remote server (cloud VM, VPS) and accessing it
over the network, `localhost:1455` on the browser side points to your local
machine — not the server. In that case, authenticate via the CLI instead:

```bash
docker exec -it moltis moltis auth login --provider openai-codex
```

The CLI opens a browser on the machine where you run the command and handles
the OAuth callback locally. Tokens are saved to the config volume and picked
up by the running gateway automatically.

### Permission denied on bind mounts

When using bind mounts, ensure the directories exist and are writable:

```bash
mkdir -p ./config ./data
chmod 755 ./config ./data
```

The container runs as user `moltis` (UID 1000). If you see permission errors,
you may need to adjust ownership:

```bash
sudo chown -R 1000:1000 ./config ./data
```
