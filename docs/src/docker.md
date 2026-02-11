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
  -v moltis-config:/home/moltis/.config/moltis \
  -v moltis-data:/home/moltis/.moltis \
  -v /var/run/docker.sock:/var/run/docker.sock \
  ghcr.io/moltis-org/moltis:latest
```

Open https://localhost:13131 in your browser and configure your LLM provider to start chatting.

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

\`\`\`bash
docker logs moltis
\`\`\`
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
    volumes:
      - ./config:/home/moltis/.config/moltis
      - ./data:/home/moltis/.moltis
      - /var/run/docker.sock:/var/run/docker.sock
```

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
  -v moltis-config:/home/moltis/.config/moltis \
  -v moltis-data:/home/moltis/.moltis \
  -v /run/user/$(id -u)/podman/podman.sock:/var/run/docker.sock \
  ghcr.io/moltis-org/moltis:latest

# Podman rootful
podman run -d \
  --name moltis \
  -p 13131:13131 \
  -p 13132:13132 \
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
  -e MOLTIS_CONFIG_DIR=/config \
  -e MOLTIS_DATA_DIR=/data \
  -v ./config:/config \
  -v ./data:/data \
  -v /var/run/docker.sock:/var/run/docker.sock \
  ghcr.io/moltis-org/moltis:latest
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
