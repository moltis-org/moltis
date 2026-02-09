# Cloud Deploy

Moltis publishes a multi-arch Docker image (`linux/amd64` and `linux/arm64`)
to `ghcr.io/moltis-org/moltis`. You can deploy it to any cloud provider that
supports container images.

## Common configuration

All cloud providers terminate TLS at the edge, so Moltis must run in plain
HTTP mode. The key settings are:

| Setting | Value | Purpose |
|---------|-------|---------|
| `--no-tls` or `MOLTIS_NO_TLS=true` | Disable TLS | Provider handles HTTPS |
| `--bind 0.0.0.0` | Bind all interfaces | Required for container networking |
| `--port <PORT>` | Listen port | Must match provider's expected internal port |
| `MOLTIS_CONFIG_DIR=/data/config` | Config directory | Persist moltis.toml, credentials |
| `MOLTIS_DATA_DIR=/data` | Data directory | Persist databases, sessions, memory |
| `MOLTIS_DEPLOY_PLATFORM` | Deploy platform | Hides local-only providers (see below) |
| `MOLTIS_PASSWORD` | Initial password | Set auth password via environment variable |

```admonish warning
**Sandbox limitation**: Most cloud providers do not support Docker-in-Docker.
The sandboxed command execution feature (where the LLM runs shell commands
inside isolated containers) will not work on these platforms. The agent will
still function for chat, tool calls that don't require shell execution, and
MCP server connections.
```

### `MOLTIS_DEPLOY_PLATFORM`

Set this to the name of your cloud provider (e.g. `flyio`, `digitalocean`,
`render`). When set, Moltis hides local-only LLM providers
(local-llm and Ollama) from the provider setup page since they cannot run
on cloud VMs. The included deploy templates for Fly.io, DigitalOcean, and
Render already set this variable.

## Fly.io

The repository includes a `fly.toml` ready to use.

### Quick start

```bash
# Install the Fly CLI if you haven't already
curl -L https://fly.io/install.sh | sh

# Launch from the repo (uses fly.toml)
fly launch --image ghcr.io/moltis-org/moltis:latest

# Set your password
fly secrets set MOLTIS_PASSWORD="your-password"

# Create persistent storage
fly volumes create moltis_data --region iad --size 1
```

### How it works

- **Image**: pulled from `ghcr.io/moltis-org/moltis:latest`
- **Port**: internal 8080, Fly terminates TLS and routes HTTPS traffic
- **Storage**: a Fly Volume mounted at `/data` persists the database, sessions,
  and memory files
- **Auto-scaling**: machines stop when idle and start on incoming requests

### Custom domain

```bash
fly certs add your-domain.com
```

Then point a CNAME to `your-app.fly.dev`.

## DigitalOcean App Platform

[![Deploy to DO](https://www.deploytodo.com/do-btn-blue.svg)](https://cloud.digitalocean.com/apps/new?repo=https://github.com/moltis-org/moltis/tree/main)

Click the button above or create an app manually:

1. Go to **Apps** > **Create App**
2. Choose **Container Image** as source
3. Set image to `ghcr.io/moltis-org/moltis:latest`
4. Set the run command: `moltis --bind 0.0.0.0 --port 8080 --no-tls`
5. Set environment variables:
   - `MOLTIS_DATA_DIR` = `/data`
   - `MOLTIS_PASSWORD` = your password
6. Set the HTTP port to `8080`

```admonish info title="No persistent disk"
DigitalOcean App Platform does not support persistent disks for image-based
services in the deploy template. Data will be lost on redeployment. For
persistent storage, consider using a DigitalOcean Droplet with Docker instead.
```

## Render

[![Deploy to Render](https://render.com/images/deploy-to-render-button.svg)](https://render.com/deploy?repo=https://github.com/moltis-org/moltis)

The repository includes a `render.yaml` blueprint. Click the button above or:

1. Go to **Dashboard** > **New** > **Blueprint**
2. Connect your fork of the Moltis repository
3. Render will detect `render.yaml` and configure the service

### Configuration details

- **Port**: Render uses port 10000 by default
- **Persistent disk**: 1 GB mounted at `/data` (included in the blueprint)
- **Environment**: set `MOLTIS_PASSWORD` in the Render dashboard under
  **Environment** > **Secret Files** or **Environment Variables**

<!-- TODO: Railway deploy does not work yet
## Railway

The repository includes a `railway.json` configuration that sets the required
environment variables (`MOLTIS_CONFIG_DIR`, `MOLTIS_DATA_DIR`,
`MOLTIS_DEPLOY_PLATFORM`) automatically.

1. Create a new project on [Railway](https://railway.com)
2. Add a service from **Docker Image**: `ghcr.io/moltis-org/moltis:latest`
3. Railway injects the `$PORT` variable automatically; the `railway.json` start
   command handles the rest
4. Set additional environment variables in the Railway dashboard:
   - `MOLTIS_PASSWORD` = your password

### Persistent storage

Railway supports persistent volumes. Add one in the service settings and mount
it at `/data`.
-->

## Authentication

On first launch, Moltis requires a password or passkey to be set. In cloud
deployments the easiest approach is to set the `MOLTIS_PASSWORD` environment
variable (or secret) before deploying. This pre-configures the password so the
setup code flow is skipped.

```bash
# Fly.io
fly secrets set MOLTIS_PASSWORD="your-secure-password"

```

For Render and DigitalOcean, set the variable in the dashboard's environment
settings.

## Health checks

All provider configs use the `/health` endpoint which returns HTTP 200 when
the gateway is ready. Configure your provider's health check to use:

- **Path**: `/health`
- **Method**: `GET`
- **Expected status**: `200`
